# Spec Delta

## Purpose

Turn raw accelerometer, gyroscope, and magnetometer samples into a stable
orientation in a documented controller frame, with explicit calibration and
fallback behavior.

## ADDED Requirements

### Requirement: Documented sensor frame

The driver SHALL use the documented controller frame as its canonical output:
`+X` right, `+Y` forward toward the trigger, and `+Z` out of the touchpad face
for both accelerometer and gyroscope. It SHALL allow the raw sensor mapping to
be reconfigured without changing consumers.

#### Scenario: Level controller

- **WHEN** the controller lies flat with the touchpad facing up and is at rest
- **THEN** accelerometer gravity is reported on the canonical `+Z` axis

#### Scenario: Yaw rotation

- **WHEN** the controller is rotated left about the canonical `+Z` axis
- **THEN** the gyroscope reports a positive `Z` rate

### Requirement: Stationary gyro-bias estimation

The driver SHALL estimate gyroscope bias only while the controller is
stationary and SHALL subtract the estimate before integration. It SHALL NOT
adapt the bias while the accelerometer indicates motion or the gyro rate is
inconsistent with rest.

#### Scenario: Bias converges at rest

- **WHEN** the controller is stationary with a small constant gyro offset for
  several hundred samples
- **THEN** the reported bias converges toward that offset and orientation drift
  is reduced

#### Scenario: Rotation does not train the bias

- **WHEN** the controller is rotated continuously
- **THEN** the bias estimate is not updated from the rotation samples

### Requirement: Stable 6-DOF orientation

The driver SHALL output a normalized quaternion and Euler angles. The output
SHALL remain finite and stable for stationary input, normal motion, timestamp
wraps, and gaps up to the configured maximum integration interval.

#### Scenario: Stationary level hold

- **WHEN** the controller is held level and stationary for many samples
- **THEN** the orientation remains near identity and contains no NaN or infinite
  values

#### Scenario: Timestamp wrap

- **WHEN** the IMU timestamp wraps from near `u32::MAX` to a small value
- **THEN** the integration step uses the wrapped delta rather than a huge gap

#### Scenario: Long packet gap

- **WHEN** a sensor notification arrives after a gap longer than the maximum
  integration interval
- **THEN** the orientation update is clamped and remains finite

### Requirement: Factory calibration handling

The driver SHALL apply a decoded `0300` factory calibration only when the record
passes validation. If the record is absent or invalid, the driver SHALL fall
back to online gyro-bias estimation and SHALL report that calibration is
unavailable.

#### Scenario: Valid factory calibration

- **WHEN** a valid calibration record is received
- **THEN** its bias and alignment terms are applied and the calibration source is
  exposed in diagnostic output

#### Scenario: Invalid factory calibration

- **WHEN** the calibration record is missing or fails validation
- **THEN** the driver continues with online bias estimation and marks factory
  calibration as unavailable

### Requirement: Magnetometer calibration and fusion

The driver SHALL expose raw magnetometer data without fusing it by default. It
SHALL fuse magnetometer data only after a calibration has been established and
SHALL allow magnetometer fusion to be disabled.

#### Scenario: Uncalibrated magnetometer

- **WHEN** no magnetometer calibration is available
- **THEN** orientation uses accelerometer and gyroscope only and the raw
  magnetometer remains available to consumers

#### Scenario: Calibrated magnetometer

- **WHEN** a valid hard/soft-iron calibration is supplied and magnetometer fusion
  is enabled
- **THEN** the orientation output uses the calibrated field and reports the
  calibration as active

### Requirement: Calibration persistence

The driver SHALL be able to persist and reload a validated magnetometer
calibration across sessions without silently applying an unvalidated profile.

#### Scenario: Reload saved calibration

- **WHEN** a previously validated calibration is present at startup
- **THEN** it is loaded and reported as active before the first sensor packet

#### Scenario: Corrupt saved calibration

- **WHEN** a saved calibration cannot be parsed or validated
- **THEN** the driver ignores it, remains in 6-DOF mode, and reports the failure
