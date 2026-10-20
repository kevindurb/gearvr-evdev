//! Regression tests over captures from the first physical unit.
//!
//! Fixtures are byte-for-byte notifications recorded on 2026-10-20 from
//! `2C:BA:BA:4F:00:14` (firmware `QD4`). `sensor-zero` and `sensor-one` are
//! derived from the real three-sample suffix, so the variable sample offset is
//! covered by real touch/button bytes.

use std::fs;
use std::path::PathBuf;

use gearvr_protocol::{parse, Command, Packet, SettingsRecord, TouchState};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct Manifest {
    unit: String,
    firmware: String,
    captured: String,
    fixtures: Vec<Fixture>,
}

#[derive(Debug, Deserialize)]
struct Fixture {
    file: String,
    kind: String,
    #[serde(default)]
    origin: String,
    #[serde(default)]
    command: String,
    #[serde(default)]
    samples: usize,
    #[serde(default)]
    buttons: u8,
    #[serde(default)]
    touch_state: String,
    #[serde(default)]
    x: u16,
    #[serde(default)]
    y: u16,
    #[serde(default)]
    temperature_c: u8,
    #[serde(default)]
    battery: u8,
    #[serde(default)]
    mag: [i16; 3],
    #[serde(default)]
    imu_interval_ms: u32,
    #[serde(default)]
    mag_interval_ms: u32,
    #[serde(default)]
    firmware: String,
    #[serde(default)]
    model: String,
    #[serde(default)]
    serial: String,
    #[serde(default)]
    button_mask: u32,
    #[serde(default)]
    touch_max: u32,
    #[serde(default)]
    temperature_centi_c: i16,
    #[serde(default)]
    tentative_groups: Vec<[i32; 3]>,
}

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn decode_hex(text: &str) -> Vec<u8> {
    let clean: String = text.chars().filter(|c| c.is_ascii_hexdigit()).collect();
    assert_eq!(clean.len() % 2, 0, "odd hex length");
    clean
        .as_bytes()
        .chunks(2)
        .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
        .collect()
}

fn touch_state(name: &str) -> TouchState {
    match name {
        "JustLifted" => TouchState::JustLifted,
        "Touching" => TouchState::Touching,
        "NotTouching" => TouchState::NotTouching,
        "Reserved" => TouchState::Reserved,
        other => panic!("unknown touch state {other}"),
    }
}

#[test]
fn captured_packets_decode_as_recorded() {
    let dir = fixtures_dir();
    let manifest: Manifest =
        serde_json::from_str(&fs::read_to_string(dir.join("manifest.json")).expect("manifest"))
            .expect("manifest parses");

    assert_eq!(manifest.unit, "2C:BA:BA:4F:00:14");
    assert_eq!(manifest.firmware, "QD4");
    assert!(!manifest.captured.is_empty());
    assert!(manifest.fixtures.iter().any(|f| f.origin == "captured"));

    let mut saw_sensor = false;
    let mut saw_zero_samples = false;

    for fixture in &manifest.fixtures {
        let bytes = decode_hex(&fs::read_to_string(dir.join(&fixture.file)).expect("fixture file"));
        match fixture.kind.as_str() {
            "ack" => {
                let Packet::Ack(command) = parse(&bytes) else {
                    panic!("{}: expected ack", fixture.file);
                };
                assert_eq!(
                    command,
                    Command::from_bytes([0x08, 0x00]),
                    "{}",
                    fixture.file
                );
                assert_eq!(&command.to_bytes()[..], &decode_hex(&fixture.command)[..]);
            }
            "sensor" => {
                saw_sensor = true;
                assert_eq!(
                    bytes.len(),
                    12 + 16 * fixture.samples,
                    "{}: sample count",
                    fixture.file
                );
                let Packet::Sensor(sensor) = parse(&bytes) else {
                    panic!("{}: expected sensor packet", fixture.file);
                };
                assert_eq!(sensor.sample_count(), fixture.samples, "{}", fixture.file);
                if fixture.samples == 0 {
                    saw_zero_samples = true;
                }
                let suffix = sensor.suffix();
                assert_eq!(suffix.buttons.bits(), fixture.buttons, "{}", fixture.file);
                assert_eq!(suffix.mag, fixture.mag, "{}", fixture.file);
                assert_eq!(
                    suffix.touchpad.state,
                    touch_state(&fixture.touch_state),
                    "{}",
                    fixture.file
                );
                assert_eq!(suffix.touchpad.x, fixture.x, "{}", fixture.file);
                assert_eq!(suffix.touchpad.y, fixture.y, "{}", fixture.file);
                assert_eq!(
                    suffix.temperature_c, fixture.temperature_c,
                    "{}",
                    fixture.file
                );
                assert_eq!(suffix.battery_percent, fixture.battery, "{}", fixture.file);
            }
            "settings" => {
                let record = SettingsRecord::from_bytes(&bytes).expect("settings record length");
                assert_eq!(record.imu_interval_ms, fixture.imu_interval_ms);
                assert_eq!(record.mag_interval_ms, fixture.mag_interval_ms);
                assert_eq!(record.firmware_ascii(), Some(fixture.firmware.as_str()));
                assert_eq!(record.model_str(), Some(fixture.model.as_str()));
                assert_eq!(record.serial_str(), Some(fixture.serial.as_str()));
                assert_eq!(record.button_mask, fixture.button_mask);
                assert_eq!(record.touch_x_max, fixture.touch_max);
                assert_eq!(record.touch_y_max, fixture.touch_max);
            }
            "calibration" => {
                let Packet::Calibration(record) = parse(&bytes) else {
                    panic!("{}: expected calibration record", fixture.file);
                };
                assert_eq!(record.temperature_centi_c(), fixture.temperature_centi_c);
                assert_eq!(record.temperature_c(), 25.0);
                // The pad bit is the only bit that is zero in all eight groups.
                assert_eq!(record.pad_bits(), [false; 8]);
                assert_eq!(
                    record.decode_21bit_tentative().as_slice(),
                    fixture.tentative_groups.as_slice()
                );
            }
            other => panic!("{}: unknown fixture kind {other}", fixture.file),
        }
    }

    assert!(saw_sensor);
    assert!(saw_zero_samples, "zero-sample packet must be covered");
}
