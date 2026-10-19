use gearvr_protocol::*;
use proptest::prelude::*;

proptest! {
    #[test]
    fn arbitrary_notifications_never_panic(data in prop::collection::vec(any::<u8>(), 0..=300)) {
        let packet = parse(&data);
        match packet {
            Packet::Ack(_) => prop_assert_eq!(data.len(), 2),
            Packet::Sensor(sensor) => {
                prop_assert!(data.len() >= 12);
                prop_assert_eq!((data.len() - 12) % 16, 0);
                prop_assert_eq!(sensor.sample_count(), (data.len() - 12) / 16);
                // Touching the suffix and every sample must not panic.
                let _ = sensor.suffix();
                for sample in sensor.samples() {
                    let _ = sample.accel_mps2();
                    let _ = sample.gyro_rads();
                }
            }
            Packet::Settings(record) => {
                prop_assert!(data.len() == 40 || data.len() == 68);
                let _ = record.firmware_ascii();
                let _ = record.model_str();
                let _ = record.serial_str();
            }
            Packet::Calibration(record) => {
                prop_assert_eq!(data.len(), 66);
                let _ = record.temperature_c();
                let _ = record.decode_21bit_tentative();
            }
            Packet::Unknown(raw) => prop_assert_eq!(raw.len(), data.len()),
        }
    }

    #[test]
    fn every_sensor_length_is_dispatched(n in 0usize..=16) {
        let data = vec![0u8; 12 + 16 * n];
        let sensor = parse(&data).as_sensor().expect("sensor packet");
        prop_assert_eq!(sensor.sample_count(), n);
        prop_assert_eq!(sensor.samples().count(), n);
    }

    #[test]
    fn every_non_sensor_length_is_unknown_or_a_record(len in 0usize..=512) {
        if len == 2 || len == 40 || len == 66 || len == 68 || (len >= 12 && (len - 12) % 16 == 0) {
            return Ok(());
        }
        let data = vec![0u8; len];
        prop_assert!(matches!(parse(&data), Packet::Unknown(_)));
    }

    #[test]
    fn timestamp_delta_is_bounded_by_u32(previous in any::<u32>(), current in any::<u32>()) {
        prop_assert_eq!(
            timestamp_delta_us(previous, current),
            current.wrapping_sub(previous)
        );
    }
}
