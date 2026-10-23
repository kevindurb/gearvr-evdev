//! Minimal `/dev/uhid` transport for the virtual DualSense.
//!
//! The kernel ABI from `<linux/uhid.h>` is encoded by hand. A dedicated reader
//! thread answers feature-report requests and drains output reports even while
//! the controller is disconnected or the backend is between ticks; SDL and
//! `hid-playstation` probe the device asynchronously and will fall back to a
//! generic pad if a feature read is not answered promptly.

use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use gearvr_core::BackendError;

use crate::dualsense::{
    calibration_report, firmware_report, pairing_report, ReportSink, FEATURE_CALIBRATION,
    FEATURE_FIRMWARE_INFO, FEATURE_PAIRING_INFO, PRODUCT_ID, PRODUCT_NAME, REPORT_DESCRIPTOR,
    VENDOR_ID,
};

const UHID_DESTROY: u32 = 1;
const UHID_START: u32 = 2;
const UHID_STOP: u32 = 3;
const UHID_OPEN: u32 = 4;
const UHID_CLOSE: u32 = 5;
const UHID_OUTPUT: u32 = 6;
const UHID_GET_REPORT: u32 = 9;
const UHID_GET_REPORT_REPLY: u32 = 10;
const UHID_CREATE2: u32 = 11;
const UHID_INPUT2: u32 = 12;
const UHID_SET_REPORT: u32 = 13;
const UHID_SET_REPORT_REPLY: u32 = 14;

const UHID_DATA_MAX: usize = 4096;
const EVENT_SIZE: usize = 4 + 128 + 64 + 64 + 2 + 2 + 4 * 4 + UHID_DATA_MAX;
const BUS_BLUETOOTH: u16 = 0x05;
const EINVAL: u16 = -22i32 as u16;

struct Features {
    calibration: Vec<u8>,
    pairing: Vec<u8>,
    firmware: Vec<u8>,
}

/// A created uhid device. Dropping it sends `UHID_DESTROY`.
pub struct UhidDevice {
    writer: Arc<Mutex<File>>,
    stop: Arc<AtomicBool>,
    reader: Option<JoinHandle<()>>,
}

impl UhidDevice {
    /// Create a Bluetooth DualSense with the given MAC address.
    pub fn open(mac: [u8; 6]) -> io::Result<Self> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open("/dev/uhid")?;
        let reader = file.try_clone()?;
        let writer = Arc::new(Mutex::new(file));
        let uniq = format_mac(mac);

        let create = create2(PRODUCT_NAME, &uniq, REPORT_DESCRIPTOR.as_slice());
        writer
            .lock()
            .map_err(|_| io::Error::other("uhid writer poisoned"))?
            .write_all(&create)?;

        let features = Arc::new(Features {
            calibration: calibration_report().to_vec(),
            pairing: pairing_report(mac).to_vec(),
            firmware: firmware_report().to_vec(),
        });
        let stop = Arc::new(AtomicBool::new(false));
        let reader_writer = writer.clone();
        let reader_stop = stop.clone();
        let handle = std::thread::spawn(move || {
            read_loop(reader, reader_writer, features, reader_stop);
        });

        Ok(Self {
            writer,
            stop,
            reader: Some(handle),
        })
    }

    pub fn send_input(&self, report: &[u8]) -> io::Result<()> {
        let mut writer = self
            .writer
            .lock()
            .map_err(|_| io::Error::other("uhid writer poisoned"))?;
        writer.write_all(&input2(report))
    }
}

impl Drop for UhidDevice {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Ok(mut writer) = self.writer.lock() {
            let _ = writer.write_all(&UHID_DESTROY.to_le_bytes());
        }
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}

fn read_loop(
    mut file: File,
    writer: Arc<Mutex<File>>,
    features: Arc<Features>,
    stop: Arc<AtomicBool>,
) {
    tracing::info!("uhid reader thread started");
    let mut buffer = [0u8; EVENT_SIZE];
    while !stop.load(Ordering::SeqCst) {
        match file.read(&mut buffer) {
            Ok(0) => break,
            Ok(size) => {
                if let Err(error) = handle_event(&buffer[..size], &features, &writer) {
                    tracing::warn!(%error, "uhid event handling failed; continuing");
                }
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => {
                tracing::warn!(%error, "uhid read failed; retrying");
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
        }
    }
    tracing::info!("uhid reader thread stopped");
}

fn handle_event(event: &[u8], features: &Features, writer: &Mutex<File>) -> io::Result<()> {
    if event.len() < 4 {
        return Ok(());
    }
    let kind = u32::from_le_bytes(event[0..4].try_into().expect("4 bytes"));
    let mut writer = writer
        .lock()
        .map_err(|_| io::Error::other("uhid writer poisoned"))?;
    match kind {
        UHID_GET_REPORT if event.len() >= 10 => {
            let id = u32::from_le_bytes(event[4..8].try_into().expect("4 bytes"));
            let report_number = event[8];
            tracing::info!(report = report_number, "uhid feature report");
            let reply = match report_number {
                FEATURE_CALIBRATION => get_report_reply(id, Some(&features.calibration)),
                FEATURE_PAIRING_INFO => get_report_reply(id, Some(&features.pairing)),
                FEATURE_FIRMWARE_INFO => get_report_reply(id, Some(&features.firmware)),
                _ => get_report_reply(id, None),
            };
            writer.write_all(&reply)?;
        }
        UHID_SET_REPORT if event.len() >= 10 => {
            let id = u32::from_le_bytes(event[4..8].try_into().expect("4 bytes"));
            writer.write_all(&set_report_reply(id))?;
        }
        UHID_OUTPUT | UHID_START | UHID_STOP | UHID_OPEN | UHID_CLOSE | UHID_DESTROY => {}
        _ => {}
    }
    Ok(())
}

/// [`ReportSink`] backed by a real uhid device.
pub struct UhidSink {
    device: UhidDevice,
}

impl UhidSink {
    pub fn open(mac: [u8; 6]) -> io::Result<Self> {
        Ok(Self {
            device: UhidDevice::open(mac)?,
        })
    }
}

impl ReportSink for UhidSink {
    fn send(&mut self, report: &[u8]) -> Result<(), BackendError> {
        self.device
            .send_input(report)
            .map_err(|error| BackendError::Other(format!("uhid: {error}")))
    }
}

fn format_mac(mac: [u8; 6]) -> String {
    format!(
        "{:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
        mac[0], mac[1], mac[2], mac[3], mac[4], mac[5]
    )
}

fn create2(name: &str, uniq: &str, descriptor: &[u8]) -> Vec<u8> {
    let mut event = vec![0u8; EVENT_SIZE];
    event[0..4].copy_from_slice(&UHID_CREATE2.to_le_bytes());
    copy_cstr(&mut event[4..132], name);
    copy_cstr(&mut event[132..196], "");
    copy_cstr(&mut event[196..260], uniq);
    event[260..262].copy_from_slice(&(descriptor.len() as u16).to_le_bytes());
    event[262..264].copy_from_slice(&BUS_BLUETOOTH.to_le_bytes());
    event[264..268].copy_from_slice(&u32::from(VENDOR_ID).to_le_bytes());
    event[268..272].copy_from_slice(&u32::from(PRODUCT_ID).to_le_bytes());
    event[272..276].copy_from_slice(&1u32.to_le_bytes());
    event[276..280].copy_from_slice(&0u32.to_le_bytes());
    event[280..280 + descriptor.len()].copy_from_slice(descriptor);
    event
}

fn copy_cstr(buffer: &mut [u8], value: &str) {
    let bytes = value.as_bytes();
    let len = bytes.len().min(buffer.len().saturating_sub(1));
    buffer[..len].copy_from_slice(&bytes[..len]);
}

fn input2(report: &[u8]) -> Vec<u8> {
    let mut event = vec![0u8; 4 + 2 + UHID_DATA_MAX];
    event[0..4].copy_from_slice(&UHID_INPUT2.to_le_bytes());
    event[4..6].copy_from_slice(&(report.len() as u16).to_le_bytes());
    let len = report.len().min(UHID_DATA_MAX);
    event[6..6 + len].copy_from_slice(&report[..len]);
    event
}

fn get_report_reply(id: u32, report: Option<&[u8]>) -> Vec<u8> {
    let mut event = vec![0u8; 4 + 4 + 2 + 2 + UHID_DATA_MAX];
    event[0..4].copy_from_slice(&UHID_GET_REPORT_REPLY.to_le_bytes());
    event[4..8].copy_from_slice(&id.to_le_bytes());
    match report {
        Some(report) => {
            let len = report.len().min(UHID_DATA_MAX);
            event[8..10].copy_from_slice(&0u16.to_le_bytes());
            event[10..12].copy_from_slice(&(len as u16).to_le_bytes());
            event[12..12 + len].copy_from_slice(&report[..len]);
        }
        None => {
            event[8..10].copy_from_slice(&EINVAL.to_le_bytes());
            event[10..12].copy_from_slice(&0u16.to_le_bytes());
        }
    }
    event
}

fn set_report_reply(id: u32) -> Vec<u8> {
    let mut event = vec![0u8; 4 + 4 + 2];
    event[0..4].copy_from_slice(&UHID_SET_REPORT_REPLY.to_le_bytes());
    event[4..8].copy_from_slice(&id.to_le_bytes());
    event[8..10].copy_from_slice(&0u16.to_le_bytes());
    event
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read_u16(bytes: &[u8], offset: usize) -> u16 {
        u16::from_le_bytes(bytes[offset..offset + 2].try_into().unwrap())
    }

    fn read_u32(bytes: &[u8], offset: usize) -> u32 {
        u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
    }

    fn cstr(bytes: &[u8]) -> &str {
        let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
        std::str::from_utf8(&bytes[..end]).unwrap()
    }

    #[test]
    fn create2_has_expected_header_and_descriptor() {
        let event = create2(
            "Wireless Controller",
            "02:47:56:52:00:01",
            &REPORT_DESCRIPTOR,
        );

        assert_eq!(read_u32(&event, 0), UHID_CREATE2);
        assert_eq!(cstr(&event[4..132]), "Wireless Controller");
        assert_eq!(cstr(&event[196..260]), "02:47:56:52:00:01");
        assert_eq!(read_u16(&event, 260), REPORT_DESCRIPTOR.len() as u16);
        assert_eq!(read_u16(&event, 262), BUS_BLUETOOTH);
        assert_eq!(read_u32(&event, 264), u32::from(VENDOR_ID));
        assert_eq!(read_u32(&event, 268), u32::from(PRODUCT_ID));
        assert_eq!(
            &event[280..280 + REPORT_DESCRIPTOR.len()],
            &REPORT_DESCRIPTOR
        );
    }

    #[test]
    fn input2_encodes_length_and_payload() {
        let report = [0x31u8; 78];
        let event = input2(&report);
        assert_eq!(read_u32(&event, 0), UHID_INPUT2);
        assert_eq!(read_u16(&event, 4), 78);
        assert_eq!(&event[6..6 + 78], &report);
    }

    #[test]
    fn get_report_reply_carries_data_or_einval() {
        let report = [0x05u8; 41];
        let event = get_report_reply(7, Some(&report));
        assert_eq!(read_u32(&event, 0), UHID_GET_REPORT_REPLY);
        assert_eq!(read_u32(&event, 4), 7);
        assert_eq!(read_u16(&event, 8), 0);
        assert_eq!(read_u16(&event, 10), 41);
        assert_eq!(&event[12..12 + 41], &report);

        let missing = get_report_reply(9, None);
        assert_eq!(read_u32(&missing, 4), 9);
        assert_eq!(read_u16(&missing, 8), EINVAL);
        assert_eq!(read_u16(&missing, 10), 0);
    }

    #[test]
    fn set_report_reply_acks() {
        let event = set_report_reply(3);
        assert_eq!(read_u32(&event, 0), UHID_SET_REPORT_REPLY);
        assert_eq!(read_u32(&event, 4), 3);
        assert_eq!(read_u16(&event, 8), 0);
    }
}
