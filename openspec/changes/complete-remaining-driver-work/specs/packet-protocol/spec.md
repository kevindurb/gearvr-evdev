# Spec Delta

## Purpose

Decode controller notifications into trustworthy button, touchpad, battery,
magnetometer, IMU, settings, and calibration data, including hostile or
truncated input.

## ADDED Requirements

### Requirement: Packet length dispatch

The driver SHALL classify a notification by its length and SHALL expose
unknown lengths as opaque data rather than failing the session. A two-byte
`08 00` or `01 00` notification SHALL be treated as a command acknowledgement.

#### Scenario: Acknowledgement packet

- **WHEN** a two-byte `08 00` notification arrives
- **THEN** it is reported as an acknowledgement of the VR-mode command

#### Scenario: Unknown notification length

- **WHEN** a notification has a length that does not match a known packet type
- **THEN** the driver reports it as unknown with its bytes and continues parsing
  later notifications

### Requirement: Sensor packet decoding

The driver SHALL decode any valid `12 + 16*N` sensor packet for `N >= 0`,
including the zero-sample packet. It SHALL parse the suffix at the variable
offset after the samples and SHALL expose every IMU sample, not only the first.

#### Scenario: Three-sample packet

- **WHEN** a 60-byte sensor packet arrives
- **THEN** the driver exposes three IMU samples, the suffix magnetometer,
  touchpad, buttons, temperature, and battery fields

#### Scenario: Zero-sample packet

- **WHEN** a 12-byte sensor packet arrives
- **THEN** the driver reports an empty sample list and still exposes the suffix
  fields without panicking

#### Scenario: Single-sample packet

- **WHEN** a 28-byte sensor packet arrives
- **THEN** the driver reads the suffix at offset 28 and exposes exactly one sample

### Requirement: Touchpad and button semantics

The driver SHALL decode the touchpad position, touching/just-lifted state, and
the trigger, Home, Back, touchpad-click, and volume button bits. Volume bits
SHALL remain available to consumers even when they are not exposed as gamepad
buttons.

#### Scenario: Touch and click

- **WHEN** the suffix reports a touching coordinate and the touchpad-click bit
- **THEN** the decoded state exposes the coordinate, the touching flag, and the
  click bit independently

#### Scenario: Just-lifted state

- **WHEN** the suffix reports the just-lifted touchpad state
- **THEN** the decoded state reports the transition without treating it as a
  fresh touch

### Requirement: Settings record decoding

The driver SHALL decode the known 40-byte and 68-byte settings records and
SHALL expose the IMU interval, magnetometer interval, firmware bytes, model,
and serial fields when present. Firmware bytes SHALL be readable as both ASCII
and a numeric value.

#### Scenario: Numeric firmware form

- **WHEN** a settings record contains the little-endian bytes `QD4\0`
- **THEN** the firmware is exposed as the ASCII string `QD4` and as the numeric
  value `3425361`

#### Scenario: Short settings record

- **WHEN** the 40-byte settings variant arrives
- **THEN** the common fields are decoded and absent model/serial fields are
  reported as unavailable rather than causing an error

### Requirement: Calibration record decoding

The driver SHALL decode the 66-byte calibration record as eight signed 21-bit
axis triples followed by a little-endian temperature, and SHALL reject records
whose padding bits or value ranges are invalid.

#### Scenario: Valid calibration record

- **WHEN** a 66-byte calibration record with zero padding bits arrives
- **THEN** the driver exposes eight axis triples and the calibration temperature

#### Scenario: Invalid calibration record

- **WHEN** a 66-byte record has a non-zero padding bit or an impossible encoding
- **THEN** the driver reports the record as invalid and does not apply it

### Requirement: Hostile input safety

The parser SHALL accept arbitrary byte slices without panicking, hanging, or
reading out of bounds. Truncated known packets SHALL be reported as invalid or
unknown rather than being partially decoded as valid.

#### Scenario: Truncated sensor packet

- **WHEN** a byte slice is shorter than the minimum sensor packet
- **THEN** parsing returns an unknown or invalid result without panic

#### Scenario: Arbitrary bytes

- **WHEN** the parser receives random bytes of arbitrary length
- **THEN** it returns a classification without panic or unbounded allocation
