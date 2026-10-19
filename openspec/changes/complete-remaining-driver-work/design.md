# Design

## Context

See `proposal.md` for motivation. The current workspace already has a pure
protocol parser, controller state, 6-DOF fusion, a command sequencer with a fake
transport, an evdev/JSON/UDP output layer, and a live BlueZ transport behind the
`bluez` feature. The missing pieces are hardware-validated session behavior,
calibration, gesture output, a Steam-visible motion device, and an installed
service.

Two constraints drive the design:

- `gearvr-protocol` must remain I/O-free and testable against byte slices.
- All hardware-dependent behavior must have a fake/synthetic test path, and
  hardware-only tasks must remain visibly unvalidated until exercised.

The OpenSpec requirements in `specs/` are the behavior contract; this document
chooses the technical approach.

## Goals / Non-Goals

**Goals:**

- Put reconnect, sleep/wake recovery, and backend lifetime behind one session
  supervisor instead of relying on process restarts.
- Make calibration and fusion decisions explicit and reversible, with a
  validated-data path and an online fallback.
- Add a Steam-visible motion path without making the evdev gamepad depend on it.
- Keep every new behavior testable with fixtures, traces, or a fake transport.

**Non-Goals:**

- A kernel or in-tree Bluetooth GATT driver.
- Firmware modification or SUOTA.
- Replacing the existing evdev backend with the DualSense backend.
- Preventing the controller's activity-based sleep.
- A generic `BUS_USB` uhid gamepad.

## Decisions

### 1. Session supervisor owns reconnect; sequencer stays purely protocol-level

The existing split remains:

- `BlueZTransport` owns D-Bus, GATT discovery, notifications, and command writes.
- `Sequencer` owns only the `0800`/`0800`-ack/`0100` state machine and timers.
- A new session supervisor in the live path owns device selection, connection
  attempts, exponential backoff, and re-entry after a link drop.

The supervisor will run an outer loop:

1. Resolve/select the device and connect.
2. Create or reuse the output backends.
3. Run the sequencer and packet loop until a link error or shutdown.
4. Call backend `on_disconnect`, reset input state, close the transport, wait
   with bounded exponential backoff, and repeat.

The output backends are created once and retained while the supervisor
reconnects, preserving Steam/udev identity. A process restart remains the last
resort after repeated failures; the systemd unit's `Restart=on-failure` stays as
a safety net.

Alternatives considered:

- Put reconnect inside `Sequencer`. Rejected: it mixes BLE lifecycle with
  protocol ordering and makes the pure state machine harder to test.
- Let systemd restart the whole process for ordinary disconnects. Rejected: it
  destroys virtual-device identity and produces a visible device flap.

### 2. Calibration decoding stays pure; application lives in fusion

The 66-byte `0300` record is decoded in `gearvr-protocol` into raw signed
21-bit triples plus temperature. The parser validates the padding bit and
range, and returns a structured invalid result rather than applying anything.

`gearvr-fusion` decides whether the calibration is usable. It receives a
validated calibration object and exposes:

- factory bias/alignment application when valid;
- online gyro-bias fallback when absent or rejected;
- a diagnostic flag describing the active source.

The 6-DOF filter remains the default. MARG is opt-in and requires a separate
magnetometer calibration (hard/soft iron and the board rotation), because the
AK09912 is on a rotated board and its raw scale/offset are still uncertain.

Tests without hardware:

- synthetic 21-bit records with known values and bad padding;
- a fixture with a known `0300` capture once one exists;
- a fusion trace that compares calibrated and online-bias drift.

Alternative considered: applying calibration directly in the parser. Rejected:
it would put policy and hardware-specific validation in a crate that must stay
pure and reusable.

### 3. Configurable frames and calibration are first-class, not hidden constants

The canonical frame stays `+X` right, `+Y` toward the trigger, `+Z` out of the
touchpad face. The axis maps already in `gearvr-fusion` become configurable from
the CLI/config file rather than compile-time constants. A documented
`pose-test` helper will:

- prompt for flat, nose-up, nose-down, left-side, and right-side poses;
- capture raw accel averages and gyro signs;
- print a candidate axis map and the residual gravity magnitude;
- never auto-apply the result without confirmation.

Calibration persistence is a small versioned file (TOML or JSON) containing:

- format version;
- mag hard-iron offset and soft-iron matrix;
- board rotation;
- source timestamps and a validity hash/checksum.

Invalid or version-mismatched files are ignored with a warning. The driver does
not silently migrate unknown formats.

Alternatives considered:

- Hardcode the `gearVRC` observed map. Rejected: the docs record conflicting
  frames and the signs are not a pure rotation.
- Fuse magnetometer data immediately. Rejected: uncalibrated hard-iron dominates
  yaw and makes the output worse than 6-DOF.

### 4. Gestures are a pure detector consumed by output backends

A gesture detector will be added to the pure/state side of the workspace, with
time supplied by the caller. It takes touchpad states and returns optional tap
or directional swipe events. The evdev backend translates those events into
discrete gamepad codes (documented in the backend); raw position and click
remain unchanged.

Initial defaults, to be tuned on hardware:

- tap: touch-to-lift under 200 ms and movement under 8 touchpad units;
- swipe: movement over 40 units within 300 ms, axis chosen by dominant delta;
- one event per gesture; no repeat while the touch remains down.

The detector is tested with synthetic touch traces, including jitter, slow
drags, diagonal movement, and lift without movement.

Alternative considered: implement gestures in the evdev backend. Rejected:
they would then be untestable without uinput and duplicated in other backends.

### 5. `Backend` gains an explicit periodic tick

Continuous DualSense reports need a timer that is independent of sensor packet
arrival. The `Backend` trait will gain a default no-op `on_tick` method and a
method describing its desired tick interval. The live loop will use the minimum
interval requested by active backends.

This keeps the evdev backend unchanged in practice, while allowing the
DualSense backend to send a neutral or last-known report every 16 ms. A missed
sensor packet cannot cause SDL to time the device out.

Alternative considered: synthesize fake sensor packets. Rejected: it would
pollute controller state and JSON output.

### 6. DualSense backend is optional, feature-gated, and testable in pieces

A new backend in `gearvr-output`, behind a `dualsense` feature, will use
`/dev/uhid` with `BUS_BLUETOOTH`. It will use the well-known DualSense report
shape, Bluetooth CRC32, and a continuous 16 ms report cadence.

The report builder and CRC are pure functions with known-vector tests. The uhid
transport sits behind a small trait so the backend can be tested with a fake
writer. The real backend uses Sony's DualSense vendor/product IDs only for the
virtual device; that is required for Steam/SDL to recognize it. The backend is
opt-in and does not run by default.

Mapping:

| Controller | DualSense field |
| --- | --- |
| Trigger | R2 |
| Home | PS |
| Back | Create |
| Touchpad click | Touchpad click |
| Touchpad position | Touchpad contact |
| Fused orientation | Gyro/accelerometer |
| Battery | Battery |

Alternatives considered:

- `BUS_USB` generic uhid pad. Rejected: SDL rejects it for advanced features.
- Patching Steam. Rejected: out of scope and unmaintainable.
- Always enabling the DualSense. Rejected: some users want the plain evdev pad
  and the virtual Sony identity is opt-in.

### 7. Deployment is a user service plus ordered udev rules

The existing `70-gearvr-evdev.rules` remains the input-node rule. Add a rule for
`/dev/uhid` access only if the system does not already grant it through the
seat/uaccess path. The systemd user unit runs the binary from `~/.local/bin`,
uses `Restart=on-failure`, and logs to the journal.

The service uses evdev by default. The DualSense backend is a documented opt-in
flag/argument, not the default, so a broken uhid path cannot prevent the gamepad
from working.

Pairing stays a desktop-agent/manual step. The driver will not register a
pairing agent in this change; it will diagnose the missing-agent case and point
to `bluetoothctl`.

Alternative considered: a system service. Rejected: it would require root and
break the `uaccess`/user-session model.

## Risks / Trade-offs

- **The `0300` layout may be wrong** → decode behind validation, compare against
  online bias, and keep 6-DOF as the fallback until a multi-unit check passes.
- **Axis convention still conflicts across prior art** → make the map
  configurable and require the pose test before changing defaults.
- **DualSense impersonation may be rejected by SDL/Steam** → build the report
  builder and CRC tests first, keep the backend opt-in, and retain evdev as the
  fallback.
- **Continuous 16 ms reports increase CPU/wakeups** → only run the tick when the
  DualSense backend is active; coalesce unchanged states.
- **BlueZ pairing agents vary by desktop** → do not implement an agent in this
  change; diagnose and document `bluetoothctl` recovery.
- **The controller sleeps by design** → reconnect with bounded backoff and never
  promise sleep prevention.
- **Gesture tuning is device- and user-dependent** → configurable thresholds and
  conservative defaults; hardware validation before enabling by default.
- **A reconnect loop can hammer a wedged adapter** → exponential backoff with a
  cap, plus an operator-visible failure after the retry budget.

## Migration Plan

1. Land the hardware-free work first: parser/calibration fixtures, gesture
   detector, report builder, fake transports, and supervisor tests.
2. Add the feature flags/config fields with defaults that preserve current
   behavior. Existing `synth`, `decode`, `info`, and evdev runs keep working.
3. Validate the evdev path on the Deck before enabling the DualSense backend by
   default in any packaging example.
4. Install the systemd unit behind a documented manual step; rollback is
   `systemctl --user disable --now gearvr.service` plus removing the unit.
5. Calibration files are additive; rollback is deleting the file, which returns
   the driver to online bias and 6-DOF.

## Open Questions

- Exact tap/swipe thresholds and gesture code assignments need hardware/user
  testing; the defaults above are placeholders and are configurable.
- Whether the `0300` layout is correct across QC1, QD4, and AQBG units remains
  unproven until captures from multiple units are compared.
- Whether the AK09912 board rotation is exactly 30 degrees or unit-specific
  remains a calibration-time question.
- Whether Steam on the target Deck build accepts the virtual DualSense without
  additional quirks cannot be answered without hardware.
