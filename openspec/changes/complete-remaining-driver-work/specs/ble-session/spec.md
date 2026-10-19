# Spec Delta

## Purpose

Establish and maintain a BLE session with the controller, including pairing,
the vendor handshake, streaming supervision, reconnect, and actionable
diagnostics.

## ADDED Requirements

### Requirement: Controller discovery and connection

The driver SHALL discover and connect to a controller selected by Bluetooth
address or by name. It SHALL report an actionable error when the controller is
not present, not paired, not trusted, or cannot be connected.

#### Scenario: Paired controller connects

- **WHEN** the selected controller is paired, trusted, and powered on
- **THEN** the driver connects and begins the vendor handshake

#### Scenario: Unpaired controller without explicit pairing

- **WHEN** the selected controller is not paired and the caller has not
  requested pairing
- **THEN** the driver exits with a message naming the controller and the
  `bluetoothctl` pair/trust recovery steps

#### Scenario: Controller absent

- **WHEN** a scan completes without finding a matching controller
- **THEN** the driver reports that the controller was not found and does not
  create a half-configured output device

### Requirement: VR-mode acknowledgement ordering

The driver SHALL write the `0800` VR-mode command, wait for the controller's
`08 00` acknowledgement, and only then write the `0100` sensor-mode command.
It SHALL NOT write `0100` before the acknowledgement unless sensor data has
already arrived.

#### Scenario: Normal handshake

- **WHEN** `0800` is written and `08 00` arrives within the acknowledgement
  window
- **THEN** the driver writes `0100` once and enters the streaming state

#### Scenario: Sensor data arrives before the acknowledgement

- **WHEN** a valid sensor notification arrives while the driver is still waiting
  for `08 00`
- **THEN** the driver treats the stream as established and still writes `0100`

#### Scenario: No acknowledgement

- **WHEN** no `08 00` acknowledgement arrives within the configured window
- **THEN** the driver retries `0800` up to the configured attempt limit and never
  writes `0100` during that wait

### Requirement: Streaming supervision and recovery

The driver SHALL monitor the sensor stream and recover when it stalls. It
SHALL send `0000` before retrying the handshake, back off between attempts, and
fall back to plain `0100` slow mode after the configured retry budget before
reporting failure.

#### Scenario: Stream stops mid-session

- **WHEN** no sensor packet arrives for the configured idle interval while
  streaming
- **THEN** the driver sends `0000`, backs off, and re-runs the `0800` handshake

#### Scenario: Retry budget exhausted

- **WHEN** the stream fails to recover after the configured retries
- **THEN** the driver attempts slow mode and reports failure only if slow mode
  also stops producing data

### Requirement: Keep-alive behavior

The driver SHALL send the `0400` keep-alive at a configurable interval while
streaming and SHALL allow keep-alives to be disabled. The driver SHALL NOT
claim that keep-alives prevent the controller's activity-based sleep.

#### Scenario: Keep-alive enabled

- **WHEN** the configured interval elapses during an active stream
- **THEN** the driver writes `0400` once and schedules the next keep-alive

#### Scenario: Keep-alive disabled

- **WHEN** keep-alives are disabled
- **THEN** no `0400` command is written

### Requirement: Reconnect after sleep or link loss

The driver SHALL return to a disconnected state when the BLE link drops, reset
all held inputs, and reconnect with bounded backoff. A successful reconnect
SHALL re-run the handshake without requiring a process restart.

#### Scenario: Mid-stream disconnect

- **WHEN** the controller disconnects while streaming
- **THEN** the driver releases held inputs and attempts to reconnect with backoff

#### Scenario: Repeated sleep and wake

- **WHEN** the controller sleeps and reconnects repeatedly
- **THEN** each session completes a fresh handshake and the driver does not
  accumulate retry state or permanent failures

### Requirement: Operator diagnostics

The driver SHALL provide a diagnostic command that reports adapter
availability, device pairing/trust state, `/dev/uinput` and `/dev/uhid` access,
and the likely recovery action for common BlueZ failures.

#### Scenario: Missing input permissions

- **WHEN** the diagnostic command cannot open `/dev/uinput`
- **THEN** it reports the permission failure and points to the udev rule and
  group requirements

#### Scenario: Pairing agent unavailable

- **WHEN** an explicit pairing attempt fails because no pairing agent is
  available
- **THEN** the diagnostic output explains how to run a pairing agent or pair
  manually
