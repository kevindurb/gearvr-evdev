//! `gearvr-cli`: offline synthesis, packet decoding, and (later) the live BLE
//! driver loop.
//!
//! The `synth` command is the hardware-free probe used to validate the virtual
//! evdev gamepad against Steam Input on a Steam Deck.

use std::io::{self, BufRead};
use std::net::SocketAddr;
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use clap::{Args, Parser, Subcommand, ValueEnum};
#[cfg(feature = "bluez")]
use gearvr_ble::bluez::{self, BlueZOptions, BlueZTransport, DeviceSelector};
#[cfg(feature = "bluez")]
use gearvr_ble::sequencer::{Event as SeqEvent, Sequencer};
use gearvr_core::{Backend, ControllerState, ImuSampleState, LinkState, Orientation};
use gearvr_fusion::Fusion;
use gearvr_output::{
    ButtonMap, EvdevBackend, EvdevConfig, JsonBackend, MotionNodeConfig, UdpBackend, UdpConfig,
};
use gearvr_protocol::{Buttons, Packet, TouchState, Touchpad};
use tracing_subscriber::EnvFilter;

#[derive(Debug, Parser)]
#[command(
    name = "gearvr-cli",
    version,
    about = "Samsung Gear VR Controller userspace driver"
)]
struct Cli {
    /// Increase log verbosity. `RUST_LOG` overrides this.
    #[arg(long, global = true)]
    verbose: bool,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Stream synthetic controller data into an output backend.
    Synth(SynthArgs),
    /// Decode hex packet(s) from `--hex` or stdin.
    Decode(DecodeArgs),
    /// Print the protocol constants and build information.
    Info,
    /// List BlueZ adapters and known devices, optionally scanning first.
    #[cfg(feature = "bluez")]
    Scan(ScanArgs),
    /// Check the local uinput/evdev/BlueZ environment.
    #[cfg(feature = "bluez")]
    Doctor,
    /// Run the live driver against a paired controller.
    #[cfg(feature = "bluez")]
    Run(RunArgs),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum BackendKind {
    Evdev,
    Json,
    Udp,
    Null,
}

#[derive(Debug, Args)]
struct OutputArgs {
    /// Output backend.
    #[arg(long, value_enum, default_value = "evdev")]
    backend: BackendKind,

    /// Create a second evdev node with raw accelerometer/gyro axes.
    #[arg(long)]
    motion: bool,

    /// Flip touchpad Y for screen-style consumers.
    #[arg(long)]
    invert_touch_y: bool,

    /// Expose the volume rocker as BTN_TRIGGER_HAPPY1/2 (off by default).
    #[arg(long)]
    volume_buttons: bool,

    /// UDP destination for `--backend udp`.
    #[arg(long, default_value = "127.0.0.1:9000")]
    udp_target: SocketAddr,
}

impl Default for OutputArgs {
    fn default() -> Self {
        Self {
            backend: BackendKind::Evdev,
            motion: false,
            invert_touch_y: false,
            volume_buttons: false,
            udp_target: "127.0.0.1:9000".parse().expect("static address"),
        }
    }
}

#[derive(Debug, Parser)]
struct SynthArgs {
    /// Seconds to run. Use 0 to run until Ctrl-C.
    #[arg(long, default_value_t = 10.0)]
    duration: f64,

    /// Sensor packets per second (hardware reports roughly 68).
    #[arg(long, default_value_t = 68.0)]
    rate: f64,

    /// IMU samples per packet (3 gives roughly 204 samples/s).
    #[arg(long, default_value_t = 3)]
    samples: usize,

    #[command(flatten)]
    output: OutputArgs,

    /// Do not emit synthetic button/touch motion.
    #[arg(long)]
    no_animate: bool,
}

#[cfg(feature = "bluez")]
#[derive(Debug, Parser)]
struct ScanArgs {
    /// Seconds to scan before listing. Use 0 to list only devices BlueZ knows.
    #[arg(long, default_value_t = 6.0)]
    seconds: f64,
}

#[cfg(feature = "bluez")]
#[derive(Debug, Parser)]
struct RunArgs {
    /// Bluetooth address (AA:BB:CC:DD:EE:FF) or a name substring.
    #[arg(default_value = "Gear VR")]
    device: String,

    /// Seconds to run. Use 0 to run until Ctrl-C.
    #[arg(long, default_value_t = 0.0)]
    duration: f64,

    /// Pair if BlueZ says the device is not paired. Needs a working pairing agent.
    #[arg(long)]
    pair: bool,

    /// Seconds to scan when the device is not already known to BlueZ.
    #[arg(long, default_value_t = 6.0)]
    scan_timeout: f64,

    #[command(flatten)]
    output: OutputArgs,
}

#[derive(Debug, Parser)]
struct DecodeArgs {
    /// Hex bytes, e.g. "08 00" or "0800". Reads stdin if omitted.
    #[arg(long)]
    hex: Option<String>,

    /// Print a compact summary instead of the full debug representation.
    #[arg(long)]
    summary: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    init_tracing(cli.verbose);

    match cli.command.unwrap_or_else(|| {
        Command::Synth(SynthArgs {
            duration: 10.0,
            rate: 68.0,
            samples: 3,
            output: OutputArgs::default(),
            no_animate: false,
        })
    }) {
        Command::Synth(args) => synth(args),
        Command::Decode(args) => decode(args),
        Command::Info => {
            println!("gearvr-cli {}", env!("CARGO_PKG_VERSION"));
            println!(
                "vendor service: 0x{:032x}",
                gearvr_protocol::VENDOR_SERVICE_UUID
            );
            println!(
                "data characteristic: 0x{:032x}",
                gearvr_protocol::DATA_CHARACTERISTIC_UUID
            );
            println!(
                "command characteristic: 0x{:032x}",
                gearvr_protocol::COMMAND_CHARACTERISTIC_UUID
            );
            Ok(())
        }
        #[cfg(feature = "bluez")]
        Command::Scan(args) => scan(args).await,
        #[cfg(feature = "bluez")]
        Command::Doctor => doctor().await,
        #[cfg(feature = "bluez")]
        Command::Run(args) => run(args).await,
    }
}

fn init_tracing(verbose: bool) {
    let default = if verbose { "debug" } else { "info" };
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(default));
    // Logs go to stderr so `--backend json` stays machine-readable on stdout.
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(io::stderr)
        .with_target(false)
        .try_init();
}

fn build_backend(args: &OutputArgs) -> Result<Box<dyn Backend>> {
    Ok(match args.backend {
        BackendKind::Evdev => {
            let button_map = if args.volume_buttons {
                ButtonMap::default().with_volume()
            } else {
                ButtonMap::default()
            };
            let config = EvdevConfig {
                motion_node: args.motion.then(MotionNodeConfig::default),
                invert_touch_y: args.invert_touch_y,
                button_map,
                ..Default::default()
            };
            Box::new(EvdevBackend::new(config))
        }
        BackendKind::Json => Box::new(JsonBackend::new(io::stdout())),
        BackendKind::Udp => Box::new(UdpBackend::new(UdpConfig::new(args.udp_target))?),
        BackendKind::Null => Box::new(NullBackend),
    })
}

#[derive(Debug, Default)]
struct NullBackend;

impl Backend for NullBackend {
    fn on_connect(&mut self, _state: &ControllerState) -> Result<(), gearvr_core::BackendError> {
        Ok(())
    }

    fn on_state(&mut self, _state: &ControllerState) -> Result<(), gearvr_core::BackendError> {
        Ok(())
    }

    fn on_disconnect(&mut self, _state: &ControllerState) -> Result<(), gearvr_core::BackendError> {
        Ok(())
    }
}

fn synth(args: SynthArgs) -> Result<()> {
    if args.duration < 0.0 || !args.duration.is_finite() {
        bail!("--duration must be a finite non-negative number");
    }
    if args.rate <= 0.0 || !args.rate.is_finite() {
        bail!("--rate must be positive");
    }
    if args.samples == 0 {
        // The real firmware does produce zero-sample packets, but for a
        // continuous probe we want at least one sample.
        bail!("--samples must be at least 1");
    }

    let mut backend = build_backend(&args.output)?;
    let mut state = ControllerState {
        link: LinkState::Connecting,
        battery_percent: 87,
        temperature_c: 29,
        ..Default::default()
    };
    backend.on_connect(&state).context("backend connect")?;

    let sample_dt = 1.0 / (args.rate * args.samples as f64);
    let packet_dt = 1.0 / args.rate;
    let start = Instant::now();
    let mut seq = 0u64;
    let mut sample_index = 0u64;
    let mut fusion = Fusion::default();

    loop {
        let elapsed = start.elapsed().as_secs_f64();
        if args.duration > 0.0 && elapsed >= args.duration {
            break;
        }

        state.seq = seq;
        state.link = LinkState::Streaming;
        state.packets_per_second = args.rate as f32;
        state.samples_per_second = (args.rate * args.samples as f64) as f32;
        state.battery_percent = 87;
        state.temperature_c = 29;
        state.mag = [120, -80, 250];
        state.buttons = if args.no_animate {
            0
        } else {
            animated_buttons(elapsed)
        };
        state.touchpad = if args.no_animate {
            Default::default()
        } else {
            animated_touch(elapsed)
        };

        state.samples.clear();
        for _ in 0..args.samples {
            let t = sample_index as f64 * sample_dt;
            let sample = synthetic_sample(t, sample_dt as f32, &mut fusion);
            state.samples.push(sample);
            sample_index += 1;
        }
        let q = fusion.orientation();
        state.orientation = Some(Orientation {
            quat: [q.w, q.x, q.y, q.z],
            euler_rads: fusion.euler_rads(),
        });
        let bias = fusion.bias();
        state.gyro_bias_rads = [bias.x, bias.y, bias.z];

        backend.on_state(&state).context("backend state")?;
        seq += 1;

        // Absolute schedule avoids accumulating sleep error.
        let target = start + Duration::from_secs_f64(seq as f64 * packet_dt);
        let now = Instant::now();
        if target > now {
            std::thread::sleep(target - now);
        }
    }

    state.link = LinkState::Disconnected;
    state.reset_inputs();
    backend
        .on_disconnect(&state)
        .context("backend disconnect")?;
    tracing::info!(
        packets = seq,
        samples = sample_index,
        "synthetic stream finished"
    );
    Ok(())
}

fn animated_buttons(t: f64) -> u8 {
    let mut buttons = 0u8;
    if (t % 3.0) < 0.4 {
        buttons |= Buttons::TRIGGER;
    }
    if (t % 7.0) > 6.3 {
        buttons |= Buttons::BACK;
    }
    if (t % 9.0) > 8.2 {
        buttons |= Buttons::TOUCHPAD_CLICK;
    }
    // The idle flag is set while no button is down.
    if buttons == 0 {
        buttons |= Buttons::IDLE;
    }
    buttons
}

fn animated_touch(t: f64) -> gearvr_core::TouchpadState {
    let phase = t * 1.2;
    let x = (160.0 + 120.0 * phase.cos()).round().clamp(0.0, 320.0) as u16;
    let y = (160.0 + 120.0 * phase.sin()).round().clamp(0.0, 320.0) as u16;
    // Leave the pad for a short window, then touch again.
    let touching = (t % 5.0) < 4.0;
    gearvr_core::TouchpadState {
        x,
        y,
        touching,
        just_lifted: false,
        valid: true,
    }
}

fn synthetic_sample(t: f64, dt: f32, fusion: &mut Fusion) -> ImuSampleState {
    // Keep the accelerometer close to 1 g so the bias estimator and Madgwick
    // correction behave as they would at rest, with a little motion.
    let accel_mps2 = [
        1.2 * (t * 2.0 * std::f64::consts::PI * 0.7).sin(),
        1.0 * (t * 2.0 * std::f64::consts::PI * 0.3).cos(),
        9.806_65 + 0.5 * (t * 2.0 * std::f64::consts::PI * 1.1).sin(),
    ];
    let gyro_rads = [
        0.15 * (t * 2.0 * std::f64::consts::PI * 0.5).sin(),
        0.10 * (t * 2.0 * std::f64::consts::PI * 0.4).cos(),
        0.20 * (t * 2.0 * std::f64::consts::PI * 0.2).sin(),
    ];

    fusion.update(
        glam::Vec3::from_array(accel_mps2.map(|v| v as f32)),
        glam::Vec3::from_array(gyro_rads.map(|v| v as f32)),
        dt,
    );

    let accel_raw = accel_mps2.map(|v| {
        ((v / gearvr_protocol::STANDARD_GRAVITY as f64) * gearvr_protocol::ACCEL_LSB_PER_G as f64)
            .round()
            .clamp(i16::MIN as f64, i16::MAX as f64) as i16
    });
    let gyro_raw = gyro_rads.map(|v| {
        (v.to_degrees() * gearvr_protocol::GYRO_LSB_PER_DPS as f64)
            .round()
            .clamp(i16::MIN as f64, i16::MAX as f64) as i16
    });

    ImuSampleState {
        timestamp_us: (t * 1_000_000.0) as u32,
        accel_raw,
        gyro_raw,
        accel_mps2: accel_mps2.map(|v| v as f32),
        gyro_rads: gyro_rads.map(|v| v as f32),
    }
}

#[cfg(feature = "bluez")]
async fn scan(args: ScanArgs) -> Result<()> {
    if args.seconds < 0.0 || !args.seconds.is_finite() {
        bail!("--seconds must be a finite non-negative number");
    }
    let adapters = bluez::list_adapters()
        .await
        .map_err(|error| anyhow::anyhow!(error))?;
    if adapters.is_empty() {
        println!("no Bluetooth adapters found");
    } else {
        println!("adapters:");
        for adapter in &adapters {
            println!(
                "  {}  {}  powered={} discoverable={}",
                adapter.id, adapter.alias, adapter.powered, adapter.discovering
            );
        }
    }

    let devices = if args.seconds > 0.0 {
        bluez::scan_for_devices(Duration::from_secs_f64(args.seconds))
            .await
            .map_err(|error| anyhow::anyhow!(error))?
    } else {
        bluez::list_devices()
            .await
            .map_err(|error| anyhow::anyhow!(error))?
    };

    println!("\ndevices ({}):", devices.len());
    for device in devices {
        let gear = device.name.as_deref().unwrap_or("").contains("Gear VR")
            || device.alias.as_deref().unwrap_or("").contains("Gear VR")
            || device
                .services
                .iter()
                .any(|uuid| uuid.as_u128() == gearvr_protocol::VENDOR_SERVICE_UUID);
        println!(
            "  {}{}  alias={:?} name={:?} paired={} trusted={} connected={} rssi={:?}",
            if gear { "* " } else { "  " },
            device.mac_address,
            device.alias,
            device.name,
            device.paired,
            device.trusted,
            device.connected,
            device.rssi
        );
        if gear {
            println!("      services: {:?}", device.services);
        }
    }
    Ok(())
}

#[cfg(feature = "bluez")]
async fn doctor() -> Result<()> {
    println!("gearvr-cli {}", env!("CARGO_PKG_VERSION"));
    println!("uinput writable: {}", can_open("/dev/uinput"));
    println!("uhid writable:   {}", can_open("/dev/uhid"));

    let event_count = std::fs::read_dir("/dev/input")
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .filter(|entry| entry.file_name().to_string_lossy().starts_with("event"))
                .count()
        })
        .unwrap_or(0);
    println!("/dev/input event nodes: {event_count}");

    match bluez::list_adapters().await {
        Ok(adapters) => {
            for adapter in adapters {
                println!(
                    "adapter {}: alias={:?} powered={} discoverable={}",
                    adapter.id, adapter.alias, adapter.powered, adapter.discovering
                );
            }
        }
        Err(error) => println!("BlueZ: unavailable ({error})"),
    }

    let steam_log = std::env::var_os("HOME")
        .map(|home| std::path::PathBuf::from(home).join(".local/share/Steam/logs/controller.txt"));
    if let Some(path) = steam_log {
        println!(
            "Steam controller log: {}",
            if path.is_file() {
                path.display().to_string()
            } else {
                format!("missing ({})", path.display())
            }
        );
    }
    Ok(())
}

#[cfg(feature = "bluez")]
fn can_open(path: &str) -> bool {
    std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .is_ok()
}

#[cfg(feature = "bluez")]
async fn apply_live(transport: &mut BlueZTransport, events: &[SeqEvent]) -> Result<()> {
    for event in events {
        match event {
            SeqEvent::Write(command) => {
                transport
                    .write_command(*command)
                    .await
                    .with_context(|| format!("write {command:?}"))?;
            }
            SeqEvent::StateChanged(state) => {
                tracing::info!(state = state.name(), "sequencer state");
            }
            SeqEvent::Error(error) => {
                bail!("sequencer error: {error}");
            }
        }
    }
    Ok(())
}

#[cfg(feature = "bluez")]
async fn run(args: RunArgs) -> Result<()> {
    if args.duration < 0.0 || !args.duration.is_finite() {
        bail!("--duration must be a finite non-negative number");
    }
    if args.scan_timeout < 0.0 || !args.scan_timeout.is_finite() {
        bail!("--scan-timeout must be a finite non-negative number");
    }

    let selector = DeviceSelector::parse(&args.device);
    let options = BlueZOptions {
        scan_timeout: Duration::from_secs_f64(args.scan_timeout),
        pair: args.pair,
        ..Default::default()
    };
    tracing::info!(?selector, "looking for controller");
    let mut transport = BlueZTransport::connect(selector, options)
        .await
        .map_err(|error| anyhow::anyhow!(error))?;

    let mut backend = build_backend(&args.output)?;
    let mut state = ControllerState {
        link: LinkState::Connecting,
        ..Default::default()
    };
    backend.on_connect(&state).context("backend connect")?;

    let start = Instant::now();
    let mut sequencer = Sequencer::default();
    let mut fusion = Fusion::default();
    let mut last_timestamp: Option<u32> = None;
    let mut last_packet: Option<Duration> = None;
    let mut seq = 0u64;
    let mut result: Result<()> = Ok(());

    let events = sequencer.start(Duration::ZERO);
    if let Err(error) = apply_live(&mut transport, &events).await {
        result = Err(error);
    }

    while result.is_ok() {
        let now = start.elapsed();
        if args.duration > 0.0 && now.as_secs_f64() >= args.duration {
            break;
        }

        let wait = sequencer
            .next_deadline()
            .map(|deadline| deadline.saturating_sub(now))
            .unwrap_or(Duration::from_millis(500));

        tokio::select! {
            signal = tokio::signal::ctrl_c() => {
                signal.context("Ctrl-C handler")?;
                break;
            }
            packet = transport.next_packet() => {
                let bytes = match packet {
                    Ok(bytes) => bytes,
                    Err(error) => {
                        result = Err(anyhow::anyhow!("link error: {error}"));
                        break;
                    }
                };
                let now = start.elapsed();
                let parsed = gearvr_protocol::parse(&bytes);

                if let Some(sensor) = parsed.as_sensor() {
                    state.apply_packet(sensor, seq);
                    seq += 1;

                    if let Some(previous) = last_packet {
                        let dt = now.saturating_sub(previous).as_secs_f32();
                        if dt > 0.0 {
                            let instantaneous = 1.0 / dt;
                            state.packets_per_second = if state.packets_per_second == 0.0 {
                                instantaneous
                            } else {
                                state.packets_per_second * 0.9 + instantaneous * 0.1
                            };
                        }
                    }
                    last_packet = Some(now);
                    state.samples_per_second =
                        state.packets_per_second * state.samples.len() as f32;

                    for sample in &state.samples {
                        let dt = last_timestamp
                            .map(|previous| {
                                gearvr_protocol::timestamp_delta_us(previous, sample.timestamp_us)
                                    as f32
                                    / 1_000_000.0
                            })
                            .unwrap_or(1.0 / 204.8);
                        last_timestamp = Some(sample.timestamp_us);
                        fusion.update(
                            glam::Vec3::from_array(sample.accel_mps2),
                            glam::Vec3::from_array(sample.gyro_rads),
                            dt,
                        );
                    }
                    let q = fusion.orientation();
                    state.orientation = Some(Orientation {
                        quat: [q.w, q.x, q.y, q.z],
                        euler_rads: fusion.euler_rads(),
                    });
                    let bias = fusion.bias();
                    state.gyro_bias_rads = [bias.x, bias.y, bias.z];
                } else {
                    match &parsed {
                        Packet::Settings(record) => tracing::info!(
                            imu_ms = record.imu_interval_ms,
                            mag_ms = record.mag_interval_ms,
                            firmware = record.firmware_ascii(),
                            model = record.model_str(),
                            serial = record.serial_str(),
                            "settings record"
                        ),
                        Packet::Calibration(record) => tracing::info!(
                            temperature_c = record.temperature_c(),
                            "calibration record"
                        ),
                        Packet::Unknown(raw) => {
                            tracing::debug!(len = raw.len(), "unknown notification");
                        }
                        Packet::Ack(command) => {
                            tracing::debug!(?command, "command ack");
                        }
                        Packet::Sensor(_) => {}
                    }
                }

                let events = sequencer.on_packet(parsed, now);
                if let Err(error) = apply_live(&mut transport, &events).await {
                    result = Err(error);
                    break;
                }
                if parsed.as_sensor().is_some() {
                    if let Err(error) = backend.on_state(&state) {
                        result = Err(error.into());
                        break;
                    }
                }
            }
            _ = tokio::time::sleep(wait) => {}
        }

        let events = sequencer.poll(start.elapsed());
        if let Err(error) = apply_live(&mut transport, &events).await {
            result = Err(error);
            break;
        }
        state.link = match sequencer.state() {
            gearvr_ble::State::Streaming | gearvr_ble::State::StreamingSlow => LinkState::Streaming,
            gearvr_ble::State::Backoff => LinkState::Backoff,
            gearvr_ble::State::Idle => LinkState::Disconnected,
            _ => LinkState::Connecting,
        };
    }

    let stop_events = sequencer.stop();
    if let Err(error) = apply_live(&mut transport, &stop_events).await {
        if result.is_ok() {
            result = Err(error);
        }
    }
    state.link = LinkState::Disconnected;
    state.reset_inputs();
    let _ = backend.on_disconnect(&state);
    let _ = transport.disconnect().await;
    result
}

fn decode(args: DecodeArgs) -> Result<()> {
    if let Some(hex) = args.hex {
        let data = parse_hex(&hex)?;
        print_packet(&data, args.summary)?;
        return Ok(());
    }

    let stdin = io::stdin();
    for line in stdin.lock().lines() {
        let line = line?;
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let data = parse_hex(line)?;
        print_packet(&data, args.summary)?;
    }
    Ok(())
}

fn print_packet(data: &[u8], summary: bool) -> Result<()> {
    let packet = gearvr_protocol::parse(data);
    if summary {
        match packet {
            Packet::Ack(command) => println!(
                "ack {} ({:02x} {:02x})",
                command.name(),
                command.to_bytes()[0],
                command.to_bytes()[1]
            ),
            Packet::Sensor(sensor) => {
                let suffix = sensor.suffix();
                println!(
                    "sensor samples={} mag={:?} touch=({}, {}) {:?} buttons={:?} battery={}%",
                    sensor.sample_count(),
                    suffix.mag,
                    suffix.touchpad.x,
                    suffix.touchpad.y,
                    suffix.touchpad.state,
                    suffix.buttons,
                    suffix.battery_percent
                );
            }
            Packet::Settings(record) => println!(
                "settings imu={}ms mag={}ms fw={:?} model={:?} serial={:?}",
                record.imu_interval_ms,
                record.mag_interval_ms,
                record.firmware_ascii(),
                record.model_str(),
                record.serial_str()
            ),
            Packet::Calibration(record) => println!(
                "calibration temp={}°C pad={:?}",
                record.temperature_c(),
                record.pad_bits()
            ),
            Packet::Unknown(raw) => println!("unknown len={} {:02x?}", raw.len(), raw),
        }
    } else {
        println!("{packet:#?}");
    }
    Ok(())
}

fn parse_hex(text: &str) -> Result<Vec<u8>> {
    let clean: String = text
        .chars()
        .filter(|c| !c.is_ascii_whitespace() && !matches!(c, ':' | ',' | '-' | '_'))
        .collect();
    let clean = clean
        .strip_prefix("0x")
        .or_else(|| clean.strip_prefix("0X"))
        .unwrap_or(&clean);
    if clean.len() % 2 != 0 {
        bail!("hex input has an odd number of digits: {clean:?}");
    }
    let mut out = Vec::with_capacity(clean.len() / 2);
    for chunk in clean.as_bytes().chunks(2) {
        let pair = std::str::from_utf8(chunk).context("hex is not utf-8")?;
        out.push(u8::from_str_radix(pair, 16).with_context(|| format!("bad hex byte {pair:?}"))?);
    }
    Ok(out)
}

/// Small helper used by `Info` and available for future fixtures.
#[allow(dead_code)]
fn describe_touchpad(touchpad: &Touchpad) -> &'static str {
    match touchpad.state {
        TouchState::Touching => "touching",
        TouchState::JustLifted => "just-lifted",
        TouchState::NotTouching => "not-touching",
        TouchState::Reserved => "reserved",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_parser_accepts_common_separators() {
        assert_eq!(parse_hex("08 00").unwrap(), vec![0x08, 0x00]);
        assert_eq!(parse_hex("08:00").unwrap(), vec![0x08, 0x00]);
        assert_eq!(parse_hex("0x0800").unwrap(), vec![0x08, 0x00]);
        assert_eq!(parse_hex("08-00,01").unwrap(), vec![0x08, 0x00, 0x01]);
    }

    #[test]
    fn hex_parser_rejects_bad_input() {
        assert!(parse_hex("0").is_err());
        assert!(parse_hex("zz").is_err());
    }

    #[test]
    fn animated_buttons_idle_when_nothing_pressed() {
        assert!(Buttons::new(animated_buttons(1.0)).idle());
        assert!(Buttons::new(animated_buttons(0.1)).trigger());
    }

    #[test]
    fn synthetic_touch_stays_in_range() {
        for i in 0..1000 {
            let touch = animated_touch(i as f64 * 0.01);
            assert!(touch.x <= 320);
            assert!(touch.y <= 320);
        }
    }
}
