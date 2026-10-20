//! `gearvr-cli`: offline synthesis, packet decoding, and (later) the live BLE
//! driver loop.
//!
//! The `synth` command is the hardware-free probe used to validate the virtual
//! evdev gamepad against Steam Input on a Steam Deck.

use std::fs::File;
use std::io::{self, BufRead, BufReader, BufWriter, Write};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use clap::{Args, Parser, Subcommand, ValueEnum};
#[cfg(feature = "bluez")]
use gearvr_ble::bluez::{self, BlueZOptions, BlueZTransport, DeviceSelector};
use gearvr_core::{Backend, ControllerState, ImuSampleState, LinkState, Orientation};
use gearvr_fusion::{fit, FitOptions, Fusion, MagCalibration};
#[cfg(feature = "dualsense")]
use gearvr_output::{dualsense::DualSenseBackend, uhid::UhidSink};
use gearvr_output::{
    ButtonMap, EvdevBackend, EvdevConfig, JsonBackend, MotionNodeConfig, UdpBackend, UdpConfig,
};
use gearvr_protocol::{Buttons, Command as ProtoCommand, Packet, TouchState, Touchpad};
use tracing_subscriber::EnvFilter;

mod supervisor;

#[cfg(feature = "bluez")]
use supervisor::{BlueZConnector, LiveSession, SessionConfig};

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
    /// Record raw controller traffic without creating an output device.
    #[cfg(feature = "bluez")]
    Capture(CaptureArgs),
    /// Read the data characteristic and send diagnostic commands, recording all replies.
    #[cfg(feature = "bluez")]
    Probe(ProbeArgs),
    /// Replay a capture file written by `run --capture` or `capture`.
    Replay(ReplayArgs),
    /// Fit a magnetometer calibration from a capture and write a TOML profile.
    MagFit(MagFitArgs),
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
    /// Virtual DualSense over Bluetooth uhid (needs the `dualsense` feature).
    #[cfg(feature = "dualsense")]
    Dualsense,
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

    /// Validated magnetometer calibration (TOML) for optional MARG fusion.
    #[arg(long, value_name = "PATH")]
    mag_calibration: Option<PathBuf>,

    /// Fuse the calibrated magnetometer for yaw correction.
    #[arg(long)]
    marg: bool,
}

impl Default for OutputArgs {
    fn default() -> Self {
        Self {
            backend: BackendKind::Evdev,
            motion: false,
            invert_touch_y: false,
            volume_buttons: false,
            udp_target: "127.0.0.1:9000".parse().expect("static address"),
            mag_calibration: None,
            marg: false,
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

    /// Write every raw notification and command write to a JSONL capture file.
    #[arg(long, value_name = "PATH")]
    capture: Option<PathBuf>,

    #[command(flatten)]
    output: OutputArgs,
}

#[cfg(feature = "bluez")]
#[derive(Debug, Parser)]
struct CaptureArgs {
    /// Bluetooth address (AA:BB:CC:DD:EE:FF) or a name substring.
    #[arg(default_value = "Gear VR")]
    device: String,

    /// Seconds to record. Use 0 to record until Ctrl-C.
    #[arg(long, default_value_t = 60.0)]
    seconds: f64,

    /// Pair if BlueZ says the device is not paired. Needs a working pairing agent.
    #[arg(long)]
    pair: bool,

    /// Seconds to scan when the device is not already known to BlueZ.
    #[arg(long, default_value_t = 6.0)]
    scan_timeout: f64,

    /// Capture file to write (JSON Lines).
    #[arg(long, short, default_value = "gearvr-capture.jsonl")]
    out: PathBuf,
}

#[cfg(feature = "bluez")]
#[derive(Debug, Parser)]
struct ProbeArgs {
    /// Bluetooth address (AA:BB:CC:DD:EE:FF) or a name substring.
    #[arg(default_value = "Gear VR")]
    device: String,

    /// Command IDs to send, in hex, in order. Defaults to `03` (calibration
    /// record) then `05` (device record).
    #[arg(long, value_delimiter = ',', default_value = "03,05")]
    commands: Vec<String>,

    /// Milliseconds to listen after each command.
    #[arg(long, default_value_t = 2000)]
    wait_ms: u64,

    /// Pair if BlueZ says the device is not paired. Needs a working pairing agent.
    #[arg(long)]
    pair: bool,

    /// Seconds to scan when the device is not already known to BlueZ.
    #[arg(long, default_value_t = 6.0)]
    scan_timeout: f64,

    /// Capture file to write (JSON Lines).
    #[arg(long, short, default_value = "gearvr-probe.jsonl")]
    out: PathBuf,
}

#[derive(Debug, Parser)]
struct ReplayArgs {
    /// Capture file written by `run --capture` or `capture`.
    path: PathBuf,

    /// Print compact one-line summaries instead of full packet debug output.
    #[arg(long)]
    summary: bool,

    /// Include command writes (tx) as well as notifications (rx).
    #[arg(long)]
    include_tx: bool,

    /// Stop after this many records. 0 prints everything.
    #[arg(long, default_value_t = 0)]
    limit: usize,
}

#[derive(Debug, Parser)]
struct MagFitArgs {
    /// Capture file written by `run --capture` or `capture`.
    capture: PathBuf,

    /// Output TOML profile.
    #[arg(long, short, default_value = "mag-calibration.toml")]
    out: PathBuf,

    /// Rotation from the magnetometer board to the controller frame.
    #[arg(long, default_value_t = -30.0)]
    board_rotation: f32,

    #[arg(long, default_value_t = 100)]
    min_samples: usize,

    /// Minimum raw spread per axis; rejects a flat capture.
    #[arg(long, default_value_t = 100.0)]
    min_range: f32,
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
        Command::Capture(args) => capture(args).await,
        #[cfg(feature = "bluez")]
        Command::Probe(args) => probe(args).await,
        Command::Replay(args) => replay(args),
        Command::MagFit(args) => mag_fit(args),
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

fn build_fusion(args: &OutputArgs) -> Result<Fusion> {
    let mut fusion = Fusion::default();
    if let Some(path) = args.mag_calibration.as_deref() {
        let calibration = MagCalibration::load(path)
            .with_context(|| format!("load magnetometer calibration {}", path.display()))?;
        tracing::info!(
            path = %path.display(),
            samples = calibration.meta.samples,
            residual = calibration.meta.residual,
            "magnetometer calibration loaded"
        );
        fusion.set_mag_calibration(Some(calibration));
    } else if args.marg {
        bail!("--marg requires --mag-calibration <PATH>");
    }
    if fusion.mag_active() {
        tracing::info!("MARG yaw correction enabled");
    }
    Ok(fusion)
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
        #[cfg(feature = "dualsense")]
        BackendKind::Dualsense => {
            // A stable locally-administered MAC keeps Steam's configuration
            // attached to the same virtual controller across restarts.
            let mac = [0x02, 0x47, 0x56, 0x52, 0x00, 0x01];
            let sink = UhidSink::open(mac).context("create virtual DualSense at /dev/uhid")?;
            Box::new(DualSenseBackend::new(Box::new(sink)))
        }
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

pub(crate) const CAPTURE_VERSION: u8 = 1;

/// One line of a capture file. `t_us` is measured from the start of the CLI
/// session, before the BLE connection is attempted, so handshake timing can be
/// read back afterwards.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub(crate) struct CaptureRecord {
    v: u8,
    t_us: u64,
    /// `rx` for a controller notification, `tx` for a command write.
    dir: String,
    /// Payload bytes as lowercase hex without separators.
    hex: String,
}

pub(crate) struct CaptureWriter {
    out: BufWriter<File>,
}

impl CaptureWriter {
    pub(crate) fn create(path: &Path) -> Result<Self> {
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("create capture directory {}", parent.display()))?;
        }
        let file = File::create(path)
            .with_context(|| format!("create capture file {}", path.display()))?;
        Ok(Self {
            out: BufWriter::new(file),
        })
    }

    pub(crate) fn record(&mut self, elapsed: Duration, dir: &str, bytes: &[u8]) -> Result<()> {
        let record = CaptureRecord {
            v: CAPTURE_VERSION,
            t_us: elapsed.as_micros().min(u64::MAX as u128) as u64,
            dir: dir.to_string(),
            hex: hex_string(bytes),
        };
        serde_json::to_writer(&mut self.out, &record).context("encode capture record")?;
        self.out.write_all(b"\n")?;
        // Flush eagerly: a disconnect or Ctrl-C must not lose the packets that
        // explain what the controller was doing.
        self.out.flush().context("flush capture file")?;
        Ok(())
    }
}

fn hex_string(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(out, "{byte:02x}");
    }
    out
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
    let mut fusion = build_fusion(&args.output)?;

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
        if fusion.mag_active() {
            fusion.update_mag(
                glam::Vec3::new(
                    state.mag[0] as f32,
                    state.mag[1] as f32,
                    state.mag[2] as f32,
                ),
                packet_dt as f32,
            );
        }
        let q = fusion.orientation();
        state.orientation = Some(Orientation {
            quat: [q.w, q.x, q.y, q.z],
            euler_rads: fusion.euler_rads(),
        });
        let bias = fusion.bias();
        state.gyro_bias_rads = [bias.x, bias.y, bias.z];

        backend.on_state(&state).context("backend state")?;
        backend.on_tick(&state).context("backend tick")?;
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

    println!(
        "gearvr udev rule: {}",
        if Path::new("/etc/udev/rules.d/70-gearvr-evdev.rules").is_file() {
            "installed"
        } else {
            "missing (install packaging/udev/70-gearvr-evdev.rules)"
        }
    );
    println!(
        "evtest/jstest:    {}/{}",
        if on_path("evtest") { "yes" } else { "no" },
        if on_path("jstest") { "yes" } else { "no" }
    );

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
fn on_path(name: &str) -> bool {
    std::env::var_os("PATH")
        .map(|paths| std::env::split_paths(&paths).any(|dir| dir.join(name).is_file()))
        .unwrap_or(false)
}

#[cfg(feature = "bluez")]
async fn capture(args: CaptureArgs) -> Result<()> {
    run(RunArgs {
        device: args.device,
        duration: args.seconds,
        pair: args.pair,
        scan_timeout: args.scan_timeout,
        capture: Some(args.out),
        output: OutputArgs {
            backend: BackendKind::Null,
            ..Default::default()
        },
    })
    .await
}

#[cfg(feature = "bluez")]
async fn probe(args: ProbeArgs) -> Result<()> {
    if args.wait_ms == 0 {
        bail!("--wait-ms must be greater than zero");
    }
    if args.scan_timeout < 0.0 || !args.scan_timeout.is_finite() {
        bail!("--scan-timeout must be a finite non-negative number");
    }
    let commands = args
        .commands
        .iter()
        .map(|text| parse_command_id(text))
        .collect::<Result<Vec<u8>>>()?;

    let origin = Instant::now();

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

    let mut capture = CaptureWriter::create(&args.out)?;
    tracing::info!(path = %args.out.display(), "recording probe traffic");

    // A plain read needs no handshake and often returns the current settings
    // record. Record it even if the device rejects the read.
    match transport.read_data().await {
        Ok(bytes) => {
            capture.record(origin.elapsed(), "read", &bytes)?;
            tracing::info!(
                len = bytes.len(),
                kind = packet_kind(&gearvr_protocol::parse(&bytes)),
                "data characteristic read"
            );
        }
        Err(error) => tracing::warn!(%error, "data characteristic read failed"),
    }

    for id in commands {
        let command = ProtoCommand::from_bytes([id, 0]);
        transport
            .write_command(command)
            .await
            .with_context(|| format!("write {command:?}"))?;
        capture.record(origin.elapsed(), "tx", &command.to_bytes())?;
        tracing::info!(?command, wait_ms = args.wait_ms, "sent probe command");

        let deadline = Instant::now() + Duration::from_millis(args.wait_ms);
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                break;
            }
            match tokio::time::timeout(remaining, transport.next_packet()).await {
                // Quiet window elapsed with no notification.
                Err(_) => break,
                Ok(Ok(bytes)) => {
                    capture.record(origin.elapsed(), "rx", &bytes)?;
                    tracing::info!(
                        len = bytes.len(),
                        kind = packet_kind(&gearvr_protocol::parse(&bytes)),
                        "notification"
                    );
                }
                Ok(Err(error)) => bail!("link error: {error}"),
            }
        }
    }

    let _ = transport.disconnect().await;
    tracing::info!(path = %args.out.display(), "probe capture written");
    Ok(())
}

#[cfg(feature = "bluez")]
fn packet_kind(packet: &Packet) -> &'static str {
    match packet {
        Packet::Ack(_) => "ack",
        Packet::Sensor(_) => "sensor",
        Packet::Settings(_) => "settings",
        Packet::Calibration(_) => "calibration",
        Packet::Unknown(_) => "unknown",
    }
}

fn parse_command_id(text: &str) -> Result<u8> {
    let clean = text.trim();
    let clean = clean
        .strip_prefix("0x")
        .or_else(|| clean.strip_prefix("0X"))
        .unwrap_or(clean);
    u8::from_str_radix(clean, 16).with_context(|| format!("bad command id {text:?}"))
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

    let capture = args
        .capture
        .as_deref()
        .map(CaptureWriter::create)
        .transpose()?;
    if let Some(path) = args.capture.as_deref() {
        tracing::info!(path = %path.display(), "recording raw traffic");
    }

    let backend = build_backend(&args.output)?;
    let connector = BlueZConnector { selector, options };
    let deadline = if args.duration > 0.0 {
        Some(Instant::now() + Duration::from_secs_f64(args.duration))
    } else {
        None
    };

    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
    tokio::spawn(async move {
        #[cfg(unix)]
        {
            use tokio::signal::unix::{signal, SignalKind};
            if let Ok(mut terminate) = signal(SignalKind::terminate()) {
                tokio::select! {
                    _ = tokio::signal::ctrl_c() => {}
                    _ = terminate.recv() => {}
                }
            } else {
                let _ = tokio::signal::ctrl_c().await;
            }
        }
        #[cfg(not(unix))]
        {
            let _ = tokio::signal::ctrl_c().await;
        }
        let _ = shutdown_tx.send(true);
    });

    let fusion = build_fusion(&args.output)?;
    let mut session = LiveSession::new(
        connector,
        backend,
        capture,
        SessionConfig::default(),
        fusion,
    );
    session.run(shutdown_rx, deadline).await
}

fn mag_fit(args: MagFitArgs) -> Result<()> {
    if args.min_samples == 0 {
        bail!("--min-samples must be greater than zero");
    }
    if !args.board_rotation.is_finite() || args.board_rotation.abs() > 180.0 {
        bail!("--board-rotation must be finite and within ±180");
    }

    let samples = load_mag_samples(&args.capture)?;
    tracing::info!(
        path = %args.capture.display(),
        samples = samples.len(),
        "magnetometer samples"
    );
    let result = fit(
        &samples,
        FitOptions {
            board_rotation_deg: args.board_rotation,
            min_samples: args.min_samples,
            min_axis_range: args.min_range,
        },
    )?;

    let calibration = &result.calibration;
    println!("samples:     {}", calibration.meta.samples);
    println!(
        "hard iron:   [{:.1}, {:.1}, {:.1}]",
        calibration.hard_iron[0], calibration.hard_iron[1], calibration.hard_iron[2]
    );
    println!(
        "axis radii:  [{:.1}, {:.1}, {:.1}]",
        result.radii[0], result.radii[1], result.radii[2]
    );
    println!(
        "soft iron:   [{:.4}, {:.4}, {:.4}]",
        calibration.soft_iron[0][0], calibration.soft_iron[1][1], calibration.soft_iron[2][2]
    );
    println!("residual:    {:.4}", calibration.meta.residual);

    calibration
        .save(&args.out)
        .with_context(|| format!("write magnetometer calibration {}", args.out.display()))?;
    println!("wrote {}", args.out.display());
    Ok(())
}

fn load_mag_samples(path: &Path) -> Result<Vec<[f32; 3]>> {
    let file = File::open(path).with_context(|| format!("open capture {}", path.display()))?;
    let mut samples = Vec::new();
    for (index, line) in BufReader::new(file).lines().enumerate() {
        let line = line?;
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let record: CaptureRecord = serde_json::from_str(line)
            .with_context(|| format!("{}:{}: invalid capture record", path.display(), index + 1))?;
        if record.dir != "rx" {
            continue;
        }
        let bytes = parse_hex(&record.hex)?;
        let packet = gearvr_protocol::parse(&bytes);
        if let Some(sensor) = packet.as_sensor() {
            let mag = sensor.suffix().mag;
            // The controller reports zeros until the first magnetometer sample.
            if mag != [0, 0, 0] {
                samples.push([mag[0] as f32, mag[1] as f32, mag[2] as f32]);
            }
        }
    }
    Ok(samples)
}

fn replay(args: ReplayArgs) -> Result<()> {
    let file = File::open(&args.path)
        .with_context(|| format!("open capture file {}", args.path.display()))?;
    let mut shown = 0usize;
    for (index, line) in BufReader::new(file).lines().enumerate() {
        let line = line?;
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let record: CaptureRecord = serde_json::from_str(line).with_context(|| {
            format!(
                "{}:{}: invalid capture record",
                args.path.display(),
                index + 1
            )
        })?;
        if !args.include_tx && record.dir == "tx" {
            continue;
        }
        if args.limit > 0 && shown >= args.limit {
            break;
        }
        let data = parse_hex(&record.hex)
            .with_context(|| format!("{}:{}: invalid hex", args.path.display(), index + 1))?;
        println!(
            "# t={}us dir={} len={}",
            record.t_us,
            record.dir,
            data.len()
        );
        print_packet(&data, args.summary)?;
        shown += 1;
    }
    Ok(())
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
                "calibration temp={:.2}°C pad={:?}",
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

    #[test]
    fn marg_requires_a_calibration() {
        let missing = OutputArgs {
            marg: true,
            ..Default::default()
        };
        assert!(build_fusion(&missing).is_err());

        let default = build_fusion(&OutputArgs::default()).unwrap();
        assert!(!default.mag_active());
    }

    #[cfg(feature = "dualsense")]
    #[test]
    fn dualsense_backend_is_selectable() {
        assert_eq!(
            <BackendKind as ValueEnum>::from_str("dualsense", true).unwrap(),
            BackendKind::Dualsense
        );
    }

    #[test]
    fn hex_string_round_trips() {
        let bytes = [0x08, 0x00, 0xff, 0x0a];
        assert_eq!(hex_string(&bytes), "0800ff0a");
        assert_eq!(parse_hex(&hex_string(&bytes)).unwrap(), bytes);
    }

    #[test]
    fn command_ids_parse_with_and_without_prefix() {
        assert_eq!(parse_command_id("03").unwrap(), 0x03);
        assert_eq!(parse_command_id("0x05").unwrap(), 0x05);
        assert_eq!(parse_command_id(" 08 ").unwrap(), 0x08);
        assert!(parse_command_id("100").is_err());
        assert!(parse_command_id("zz").is_err());
    }

    #[test]
    fn capture_writer_round_trips() {
        let path =
            std::env::temp_dir().join(format!("gearvr-capture-test-{}.jsonl", std::process::id()));
        {
            let mut writer = CaptureWriter::create(&path).unwrap();
            writer
                .record(Duration::from_millis(1), "tx", &[0x08, 0x00])
                .unwrap();
            writer
                .record(Duration::from_micros(1500), "rx", &[0x08, 0x00])
                .unwrap();
        }
        let text = std::fs::read_to_string(&path).unwrap();
        let _ = std::fs::remove_file(&path);

        let records: Vec<CaptureRecord> = text
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(records.len(), 2);
        assert_eq!(records[0].v, CAPTURE_VERSION);
        assert_eq!(records[0].dir, "tx");
        assert_eq!(records[0].hex, "0800");
        assert_eq!(records[1].t_us, 1500);
    }
}
