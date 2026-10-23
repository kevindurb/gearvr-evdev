//! Live BlueZ transport, behind the `bluez` feature.
//!
//! This module owns the D-Bus session and the two vendor characteristics. It
//! deliberately keeps the command sequencer outside: the sequencer decides
//! *what* to write, this module only performs the write and yields
//! notifications.
//!
//! The controller is paired/bonded by the system Bluetooth stack. We do not
//! register a pairing agent; that is desktop-environment territory and is a
//! common source of surprising prompts.

use std::pin::Pin;
use std::time::Duration;

use bluez_async::{
    BluetoothEvent, BluetoothSession, CharacteristicEvent, CharacteristicFlags, CharacteristicId,
    DeviceEvent, DeviceId, DeviceInfo, DiscoveryFilter, MacAddress, Transport, WriteOptions,
    WriteType,
};
use futures::{Stream, StreamExt};
use gearvr_protocol::{
    Command, COMMAND_CHARACTERISTIC_UUID, DATA_CHARACTERISTIC_UUID, VENDOR_SERVICE_UUID,
};
use tokio::task::JoinHandle;
use uuid::Uuid;

use crate::transport::TransportError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeviceSelector {
    Address(String),
    /// Case-insensitive substring match against the alias or name.
    Name(String),
    /// Pick the first device advertising the vendor service.
    Any,
}

impl DeviceSelector {
    pub fn parse(value: &str) -> Self {
        if value.contains(':') || value.contains('-') {
            Self::Address(normalize_address(value))
        } else {
            Self::Name(value.to_string())
        }
    }

    fn matches(&self, device: &DeviceInfo) -> bool {
        match self {
            Self::Any => device
                .services
                .contains(&Uuid::from_u128(VENDOR_SERVICE_UUID)),
            Self::Address(address) => {
                normalize_address(&device.mac_address.to_string()) == *address
            }
            Self::Name(name) => {
                let needle = name.to_ascii_lowercase();
                let alias = device.alias.as_deref().unwrap_or("");
                let name = device.name.as_deref().unwrap_or("");
                alias.to_ascii_lowercase().contains(&needle)
                    || name.to_ascii_lowercase().contains(&needle)
            }
        }
    }
}

fn normalize_address(address: &str) -> String {
    address.replace('-', ":").to_ascii_uppercase()
}

/// Aborts the D-Bus driver task when dropped.
///
/// Every failed connection attempt must drop this, otherwise its D-Bus socket
/// stays open and a long-running reconnect loop eventually exhausts the fd
/// limit.
struct DbusTask(JoinHandle<()>);

impl Drop for DbusTask {
    fn drop(&mut self) {
        self.0.abort();
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlueZOptions {
    pub scan_timeout: Duration,
    pub connect_timeout: Duration,
    pub pair_timeout: Duration,
    /// Attempt `Pair()` when the device is not already paired.
    pub pair: bool,
}

impl Default for BlueZOptions {
    fn default() -> Self {
        Self {
            scan_timeout: Duration::from_secs(6),
            connect_timeout: Duration::from_secs(15),
            pair_timeout: Duration::from_secs(35),
            pair: false,
        }
    }
}

/// A connected controller. Drop does not disconnect; call [`Self::disconnect`].
pub struct BlueZTransport {
    session: BluetoothSession,
    device: DeviceId,
    data: CharacteristicId,
    command: CharacteristicId,
    write_type: WriteType,
    events: Pin<Box<dyn Stream<Item = BluetoothEvent> + Send>>,
    dbus_task: DbusTask,
}

impl std::fmt::Debug for BlueZTransport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BlueZTransport")
            .field("device", &self.device)
            .field("data", &self.data)
            .field("command", &self.command)
            .field("write_type", &self.write_type)
            .finish_non_exhaustive()
    }
}

impl BlueZTransport {
    /// Connect to a controller. If the selector is not already known to BlueZ,
    /// scan for it first.
    pub async fn connect(
        selector: DeviceSelector,
        options: BlueZOptions,
    ) -> Result<Self, TransportError> {
        let (dbus, session) = BluetoothSession::new()
            .await
            .map_err(|error| TransportError::Other(format!("D-Bus/BlueZ unavailable: {error}")))?;
        let dbus_task = DbusTask(tokio::spawn(async move {
            if let Err(error) = dbus.await {
                tracing::error!(%error, "D-Bus connection lost");
            }
        }));

        let device = find_device(&session, &selector).await?;
        let device = match device {
            Some(device) => device,
            None => {
                scan_for(&session, &selector, options.scan_timeout).await?;
                find_device(&session, &selector).await?.ok_or_else(|| {
                    TransportError::Other(format!("controller not found after scan: {selector:?}"))
                })?
            }
        };

        if options.pair && !device.paired {
            tracing::info!(address = %device.mac_address, "pairing controller");
            session
                .pair_with_timeout(&device.id, options.pair_timeout)
                .await
                .map_err(|error| {
                    TransportError::Other(format!(
                        "pairing failed: {error}; run `bluetoothctl` and pair/trust the controller"
                    ))
                })?;
        } else if !device.paired {
            return Err(TransportError::Other(format!(
                "{} is not paired; run `bluetoothctl` (pair/trust) or pass --pair",
                device.mac_address
            )));
        }

        if !device.connected || !device.services_resolved {
            session
                .connect_with_timeout(&device.id, options.connect_timeout)
                .await
                .map_err(|error| TransportError::Other(format!("connect failed: {error}")))?;
        }
        if !device.trusted {
            tracing::warn!(
                address = %device.mac_address,
                "device is not trusted in BlueZ; reconnect may require a desktop agent"
            );
        }

        let service = session
            .get_service_by_uuid(&device.id, Uuid::from_u128(VENDOR_SERVICE_UUID))
            .await
            .map_err(|error| TransportError::Other(format!("vendor service missing: {error}")))?;
        let data_info = session
            .get_characteristic_by_uuid(&service.id, Uuid::from_u128(DATA_CHARACTERISTIC_UUID))
            .await
            .map_err(|error| {
                TransportError::Other(format!("data characteristic missing: {error}"))
            })?;
        let command_info = session
            .get_characteristic_by_uuid(&service.id, Uuid::from_u128(COMMAND_CHARACTERISTIC_UUID))
            .await
            .map_err(|error| {
                TransportError::Other(format!("command characteristic missing: {error}"))
            })?;

        if !data_info
            .flags
            .intersects(CharacteristicFlags::NOTIFY | CharacteristicFlags::INDICATE)
        {
            return Err(TransportError::Other(
                "data characteristic does not support notify/indicate".to_string(),
            ));
        }
        let write_type = if command_info
            .flags
            .contains(CharacteristicFlags::WRITE_WITHOUT_RESPONSE)
        {
            WriteType::WithoutResponse
        } else if command_info.flags.contains(CharacteristicFlags::WRITE) {
            WriteType::WithResponse
        } else {
            return Err(TransportError::Other(
                "command characteristic is not writable".to_string(),
            ));
        };

        let events = session
            .characteristic_event_stream(&data_info.id)
            .await
            .map_err(|error| {
                TransportError::Other(format!("notification stream failed: {error}"))
            })?;
        session
            .start_notify(&data_info.id)
            .await
            .map_err(|error| TransportError::Other(format!("start notify failed: {error}")))?;

        tracing::info!(
            address = %device.mac_address,
            data = %data_info.id,
            command = %command_info.id,
            ?write_type,
            "connected to controller"
        );

        Ok(Self {
            session,
            device: device.id,
            data: data_info.id,
            command: command_info.id,
            write_type,
            events: Box::pin(events),
            dbus_task,
        })
    }

    pub fn device_id(&self) -> &DeviceId {
        &self.device
    }

    pub async fn write_command(&mut self, command: Command) -> Result<(), TransportError> {
        let options = WriteOptions {
            write_type: Some(self.write_type),
            ..Default::default()
        };
        self.session
            .write_characteristic_value_with_options(
                &self.command,
                command.to_bytes().to_vec(),
                options,
            )
            .await
            .map_err(|error| TransportError::Other(format!("command write failed: {error}")))
    }

    /// Read the data characteristic once. Useful for initial settings records.
    pub async fn read_data(&self) -> Result<Vec<u8>, TransportError> {
        self.session
            .read_characteristic_value(&self.data)
            .await
            .map_err(|error| TransportError::Other(format!("data read failed: {error}")))
    }

    /// Wait for the next notification from the data characteristic.
    pub async fn next_packet(&mut self) -> Result<Vec<u8>, TransportError> {
        while let Some(event) = self.events.next().await {
            match event {
                BluetoothEvent::Characteristic {
                    id,
                    event: CharacteristicEvent::Value { value },
                } if id == self.data => return Ok(value),
                BluetoothEvent::Device {
                    id,
                    event: DeviceEvent::Connected { connected: false },
                } if id == self.device => return Err(TransportError::Disconnected),
                _ => {}
            }
        }
        Err(TransportError::Disconnected)
    }

    pub async fn disconnect(self) -> Result<(), TransportError> {
        let _ = self.session.stop_notify(&self.data).await;
        let _ = self.session.disconnect(&self.device).await;
        self.dbus_task.0.abort();
        Ok(())
    }
}

pub async fn list_devices() -> Result<Vec<DeviceInfo>, TransportError> {
    let (dbus, session) = BluetoothSession::new()
        .await
        .map_err(|error| TransportError::Other(format!("D-Bus/BlueZ unavailable: {error}")))?;
    let _dbus_task = DbusTask(tokio::spawn(async move {
        if let Err(error) = dbus.await {
            tracing::error!(%error, "D-Bus connection lost");
        }
    }));
    session
        .get_devices()
        .await
        .map_err(|error| TransportError::Other(error.to_string()))
}

/// Start a filtered LE scan, wait for `timeout`, stop, and return everything
/// BlueZ now knows about.
pub async fn scan_for_devices(timeout: Duration) -> Result<Vec<DeviceInfo>, TransportError> {
    let (dbus, session) = BluetoothSession::new()
        .await
        .map_err(|error| TransportError::Other(format!("D-Bus/BlueZ unavailable: {error}")))?;
    let _task = DbusTask(tokio::spawn(async move {
        if let Err(error) = dbus.await {
            tracing::error!(%error, "D-Bus connection lost");
        }
    }));
    let filter = DiscoveryFilter {
        service_uuids: vec![Uuid::from_u128(VENDOR_SERVICE_UUID)],
        transport: Some(Transport::Le),
        duplicate_data: Some(false),
        ..Default::default()
    };
    let result = async {
        session
            .start_discovery_with_filter(&filter)
            .await
            .map_err(|error| TransportError::Other(format!("scan failed: {error}")))?;
        tokio::time::sleep(timeout).await;
        let _ = session.stop_discovery().await;
        session
            .get_devices()
            .await
            .map_err(|error| TransportError::Other(error.to_string()))
    }
    .await;
    // `_task` aborts its D-Bus driver when it drops here.
    result
}

pub async fn list_adapters() -> Result<Vec<bluez_async::AdapterInfo>, TransportError> {
    let (dbus, session) = BluetoothSession::new()
        .await
        .map_err(|error| TransportError::Other(format!("D-Bus/BlueZ unavailable: {error}")))?;
    let _dbus_task = DbusTask(tokio::spawn(async move {
        if let Err(error) = dbus.await {
            tracing::error!(%error, "D-Bus connection lost");
        }
    }));
    session
        .get_adapters()
        .await
        .map_err(|error| TransportError::Other(error.to_string()))
}

async fn find_device(
    session: &BluetoothSession,
    selector: &DeviceSelector,
) -> Result<Option<DeviceInfo>, TransportError> {
    let devices = session
        .get_devices()
        .await
        .map_err(|error| TransportError::Other(error.to_string()))?;
    Ok(devices.into_iter().find(|device| selector.matches(device)))
}

async fn scan_for(
    session: &BluetoothSession,
    selector: &DeviceSelector,
    timeout: Duration,
) -> Result<(), TransportError> {
    // A name/address selector should still work if the controller omits its
    // service UUID from the advertisement. Only `Any` relies on the UUID.
    let service_uuids = if matches!(selector, DeviceSelector::Any) {
        vec![Uuid::from_u128(VENDOR_SERVICE_UUID)]
    } else {
        Vec::new()
    };
    let filter = DiscoveryFilter {
        service_uuids,
        transport: Some(Transport::Le),
        duplicate_data: Some(false),
        ..Default::default()
    };
    session
        .start_discovery_with_filter(&filter)
        .await
        .map_err(|error| TransportError::Other(format!("scan failed: {error}")))?;
    tracing::info!(
        ?selector,
        seconds = timeout.as_secs_f32(),
        "scanning for controller"
    );
    tokio::time::sleep(timeout).await;
    let _ = session.stop_discovery().await;
    Ok(())
}

/// Convert a BlueZ MAC address to the canonical string used by `--address`.
pub fn address_string(address: MacAddress) -> String {
    normalize_address(&address.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn address_selector_normalizes() {
        assert_eq!(
            DeviceSelector::parse("aa-bb-cc-dd-ee-ff"),
            DeviceSelector::Address("AA:BB:CC:DD:EE:FF".to_string())
        );
        assert_eq!(
            DeviceSelector::parse("aa:bb:cc:dd:ee:ff"),
            DeviceSelector::Address("AA:BB:CC:DD:EE:FF".to_string())
        );
        assert_eq!(
            DeviceSelector::parse("Gear VR"),
            DeviceSelector::Name("Gear VR".to_string())
        );
    }
}
