# Proposal

## Why

The repository has a tested hardware-free skeleton, but it is not yet a driver
someone can install and rely on: the live BLE path has never spoken to a real
controller, axis and calibration data are unverified, touchpad gestures and
Steam-side gyro are missing, and the systemd unit has not run on a Deck. This
change closes the gap between "builds and passes tests" and "validated,
installable driver" as one coordinated completion effort.

## What Changes

- Bring up the live controller path on real hardware: pairing/bonding, GATT
  connect, the `0800` → ack → `0100` handshake, notification decoding, writes,
  keep-alive, and recovery after sleep or a mid-stream disconnect.
- Harden the protocol layer with recorded fixtures and real calibration data:
  decode the `0300` record, validate settings/firmware variants, and keep
  malformed, truncated, and zero-sample packets safe.
- Resolve the motion frame and calibration story: confirm the axis convention
  and signs with a scripted pose test, apply factory calibration when it is
  trustworthy, and add magnetometer calibration/MARG fusion only when the data
  supports it.
- Complete the output layer: touchpad tap/swipe synthesis, clearly defined
  motion-node units and ranges, stable reconnect behavior, and a Bluetooth
  DualSense `uhid` backend so Steam Input can expose gyro.
- Finish deployment and operations: installable udev rules, a tested systemd
  user service, pairing/recovery documentation, and diagnostics that identify
  the common BlueZ failure modes.
- Record the results in `docs/implementation.md`, including which claims are
  hardware-validated and which remain assumptions.

## Capabilities

### New Capabilities

- `ble-session`: connection, pairing/bonding checks, handshake sequencing,
  keep-alive, reconnect/backoff, and operator-facing diagnostics.
- `packet-protocol`: packet parsing and decoding, including calibration and
  settings records, firmware variants, and hostile-input safety.
- `motion-fusion`: axis mapping, gyro-bias handling, factory calibration,
  magnetometer calibration, and 6-DOF/MARG orientation output.
- `output-backends`: evdev gamepad and motion nodes, touchpad gestures,
  JSON/UDP, and the Bluetooth DualSense `uhid` backend.
- `deployment`: udev permissions, systemd user service behavior, install and
  recovery documentation, and log/journal expectations.

### Modified Capabilities

None. The project has no existing OpenSpec capabilities; these are the first
main specs for the driver.

## Impact

- `crates/gearvr-ble`: live transport behavior, reconnect supervisor, health
  diagnostics, and hardware timing.
- `crates/gearvr-protocol`: fixture parsing, calibration layout, and defensive
  handling of firmware/settings variants.
- `crates/gearvr-fusion`: axis configuration, calibration application, and
  optional magnetometer fusion.
- `crates/gearvr-output`: gesture synthesis, motion-node contract, and a new
  `uhid` backend.
- `crates/gearvr-cli`: live commands, diagnostics, pose/calibration helpers,
  and backend selection.
- `packaging/`, `scripts/`, and `docs/`: install path, service behavior,
  validation results, and operator guidance.
- New test assets: recorded notification fixtures, synthetic traces, and
  hardware-in-the-loop checklists.

## Assumptions

- "Everything not done" means the open items in `docs/implementation.md` plus
  DESIGN milestones M1–M7. Research-only unknowns such as the exact IMU or
  touchpad-controller part number are only pursued if they block a driver
  behavior.
- A physical ET-YO324 will be available for the hardware-dependent tasks. Those
  tasks stay unchecked until actually exercised; the change must still land all
  hardware-free work first.
- Steam-side gyro is required through the `uhid` DualSense path, not by
  pretending a generic evdev axis is gyro.

## Non-goals

- A kernel module or in-tree Bluetooth GATT driver.
- SUOTA/firmware modification.
- Windows/macOS support, haptics, or 6-DOF positional tracking.
- A generic `BUS_USB` `uhid` gamepad; the DualSense backend uses
  `BUS_BLUETOOTH` because that is the path SDL/Steam accepts.
- Preventing the controller's activity-based sleep; the driver designs around
  it.
