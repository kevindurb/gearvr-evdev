# Implementation status and validation

This document records what is actually built, what was measured on the Steam
Deck, and what remains blocked on hardware. `DESIGN.md` is the plan; this is the
current state.

## Crates

| Crate | Status | Notes |
| --- | --- | --- |
| `gearvr-protocol` | Implemented | Pure parser, no I/O, `no_std`-capable. Packet length dispatch, IMU samples, suffix, buttons, touchpad, settings, tentative calibration, synthetic builders, proptests. |
| `gearvr-core` | Implemented | `ControllerState`, link state, serde mirror types, `Backend` trait. |
| `gearvr-fusion` | Implemented | Configurable axis map, stationary gyro-bias estimator, 6-DOF Madgwick. |
| `gearvr-ble` | Implemented (hardware untested) | Pure command sequencer plus fake transport; feature-gated `bluez-async` transport for scan/connect/notify/write. |
| `gearvr-output` | Implemented | evdev/uinput gamepad, optional motion node, newline JSON, JSON/UDP, and an opt-in uhid DualSense backend. |
| `gearvr-cli` | Implemented | `synth`, `decode`, `info`, `scan`, `doctor`, `capture`, `probe`, `replay`, a reconnecting session supervisor, and a live `run` loop. |

`cargo test --workspace` currently runs 114 tests: 34 protocol unit tests, 1
real-capture fixture test, 4 protocol property tests, 14 fusion, 19 BLE, 27
output, 4 core, and 11 CLI. `cargo clippy --workspace --all-targets -- -D
warnings` is clean.

## Protocol decisions that differ from the obvious implementation

- The `0800` acknowledgement can take ~1.5 s and has been observed near 3 s. The
  sequencer waits **3 s** before retrying. Retrying at 100–500 ms can corrupt the
  handshake.
- `0100` is not treated as acknowledged. After the `0800` echo, sensor mode is
  written and the link enters `Streaming` immediately.
- If the stream never starts, the sequencer retries with `0000` (off), backs off,
  and re-runs the handshake. After `max_stream_retries`, it falls back to plain
  `0100` slow mode before giving up.
- A sensor packet arriving before the `0800` echo counts as stronger evidence
  than the echo; the sequencer enters streaming and still writes `0100`.
- Keep-alives are enabled at 60 s by default, but they are optional and do not
  prevent the controller's activity-based sleep.
- The suffix is parsed at `12 + 16*N`, not at a fixed offset. The magnetometer is
  at suffix offset 32 (packet offset 48 for three samples), which is the bug in
  several prior drivers.
- Firmware `QD4\0` is little-endian ASCII `"QD4"`; the numeric value `3425361`
  is the same bytes interpreted as an integer.

## Evdev mapping

Generic gamepad, `BUS_USB`, VID/PID `1209:0001` (pid.codes test IDs):

| Controller input | Kernel event |
| --- | --- |
| Trigger | `BTN_SOUTH` |
| Back | `BTN_SELECT` |
| Home | `BTN_MODE` |
| Touchpad click | `BTN_THUMBL` |
| Touchpad position | `ABS_X`, `ABS_Y`, range 0–320 |
| Volume up/down | off by default; `--volume-buttons` maps them to `BTN_TRIGGER_HAPPY1/2` |

The optional motion node is `1209:0002` with `INPUT_PROP_ACCELEROMETER`,
`ABS_X/Y/Z` for raw accelerometer counts and `ABS_RX/RY/RZ` for raw gyro counts.
It is a separate evdev device because a gamepad has no standard motion axes.

The udev rule is `packaging/udev/70-gearvr-evdev.rules`. The `70-` prefix
matters: `71-seat.rules` converts the `uaccess` tag into a seat tag, and
`73-seat-late.rules` applies the ACL. A `99-` rule is too late and leaves the
motion node unreadable by the logged-in user.

## Raw captures

`capture` and `probe` write JSON Lines, one object per event:

| Field | Meaning |
| --- | --- |
| `v` | schema version, currently `1` |
| `t_us` | microseconds since the CLI started, before the BLE connect |
| `dir` | `tx` command write, `rx` notification, `read` characteristic read |
| `hex` | payload bytes, lowercase, no separators |

`replay capture.jsonl --include-tx --summary` decodes a file offline. Records are
flushed as they are written, so Ctrl-C or a link drop keeps everything up to the
failure. These files are the input for the protocol fixtures in task 1.1.

## Steam Deck validation (2026-10-19)

Target: Bazzite 44 / Fedora 44, kernel 7.2.8, BlueZ 5.87, Steam running with
Steam Input active. The synthetic stream was produced by:

```sh
gearvr-cli synth --duration 40 --rate 68 --samples 3 --backend evdev --motion
```

Results:

- The gamepad appeared as `/dev/input/event8`, `bus 0x3 vendor 0x1209 product
  0x1`, name `"Gear VR Controller"`.
- udev tagged it `ID_INPUT=1` and `ID_INPUT_JOYSTICK=1`; `evtest` showed
  `EV_KEY` (six buttons) and `EV_ABS` (`ABS_X`, `ABS_Y`).
- `jstest` saw 2 axes and 4 buttons by default (6 when `--volume-buttons` is enabled), and reported trigger/back transitions.
- `evtest` confirmed `BTN_SOUTH` press/release and `BTN_SELECT` press/release
  from the synthetic animation.
- Steam logged the device as `type: 1209 0001`, product `Gear VR Controller`,
  and generated:

  ```text
  SDL Mapping for 1209/1: ...,Gear VR Controller,a:b0,back:b1,guide:b2,leftstick:b3,leftx:a0,lefty:a1,platform:Linux,
  ```

  It then loaded `configset_controller_generic.vdf`. This directly contradicts
  the worst-case reading of Caveat 2 in `linux-integration.md` on this Steam
  build: a generic uinput pad was enumerated and accepted by Steam Input.
- Caveat 1 still applies: the gamepad node has no gyro, and Steam Input has no
  gyro source for a generic pad. Gyro in Steam still requires the `uhid`
  DualSense prototype (or a Steam/OpenXR-side provider).
- The motion node is readable by the desktop user after installing the `70-`
  udev rule; without it, `/dev/input/event19` had no ACL and `evtest` returned
  `Permission denied`.

Packaging notes (udev ordering, pairing, systemd) are in
`packaging/README.md`. Reproduce the evdev probe with `scripts/deck-probe.sh`
after copying the release binary to
`~/gearvr-probe/gearvr-cli` on the Deck. Install the udev rule with:

```sh
sudo cp packaging/udev/70-gearvr-evdev.rules /etc/udev/rules.d/
sudo udevadm control --reload-rules
sudo udevadm trigger --subsystem-match=input
```

## Physical controller validation (2026-10-20)

Unit `2C:BA:BA:4F:00:14`, firmware `QD4`, model `ET-YO324`, serial
`RF7JB2ACG1D`. Host: `steamdeck`, Bazzite 44, kernel 7.2.8, BlueZ 5.87. Raw
traffic was recorded with `capture`/`probe`; representative packets are checked
in under `crates/gearvr-protocol/tests/fixtures/`.

Pairing used the documented `bluetoothctl` recipe. The unit reports
Paired/Bonded/Trusted and a battery level in the standard `0x180F` service.

- **Service UUID bug.** The first connection failed with "vendor service
  missing": `VENDOR_SERVICE_UUID` had two transposed bytes and decoded to
  `Ocutls T...` instead of `Oculus Threemote`. Fixed, with a byte-level
  regression test. The service then resolved at
  `service0030`, characteristics `char0031` (notify) and `char0034` (write with
  response).
- **Handshake timing.** `0800` → echo took **1.02 s** in the first session and
  **1.51 s** in the second; `0100` was written 50–54 ms after each echo. A 3 s
  timeout is required; a 1 s timeout would have failed the second session.
- **Stream.** 66.9 packets/s, 3 samples per packet, 60 bytes. Sample-timestamp
  medians were 4.825 ms (≈207 Hz), matching the 5 ms IMU interval.
- **Buttons.** Trigger `0x01`, Home `0x02`, Back `0x04`, touchpad click `0x08`.
  The idle bit `0x40` clears while a button is held and returns afterwards.
  Volume up/down were not exercised.
- **Touchpad.** All four `SS` states were observed; `JustLifted` appears for
  exactly one packet. A circular drag reported x=32..265, y=35..308.
- **Magnetometer.** Confirmed at packet offset 48 (suffix offset 0). The first
  ~10 packets after streaming starts report zero, then values settle near
  `(3240, -7845, -1240)` raw for this unit. That offset is unit-specific hard
  iron, not an Earth-field reading.
- **Gravity and axis frame (four-pose test).** Confirmed the documented
  right-handed frame:
  - flat touchpad-up: `(21, 449, 2037)`, so `+Z ≈ +1 g` (out of the pad face);
  - trigger end raised: `(225, 2034, -56)`, so `+Y` points toward the trigger;
  - left edge down: `(2044, 192, -112)`, so `+X` points right;
  - nose-up transition: gyro `+X`; left-edge-down: gyro `+Y`; yaw left: gyro
    `+Z`, all matching the right-hand rule.
  The raw sensor frame is already the canonical frame, so
  `AxisConfig::DOCUMENTED` (identity) is correct and needs no remap.
- **Gyro bias.** At rest `(4, -51, 14)` raw, a small constant bias the online
  estimator can learn.
- **evdev and Steam.** `evtest` recorded two clean `BTN_SOUTH` press/release
  pairs from the trigger and declared `BTN_SELECT`, `BTN_MODE`, and
  `BTN_THUMBL`. During the same real-input session Steam logged
  `Product: Gear VR Controller` and generated
  `a:b0,back:b1,guide:b2,leftstick:b3,leftx:a0,lefty:a1`.
- **Idle sleep (informal).** The controller dropped the link after roughly 3.5
  minutes without button activity and reconnected on the next wake. This matches
  the documented inactivity sleep, but long-duration sleep behavior is still
  unmeasured.
- **Reconnect and input release (ten cycles).** Ten forced
  `bluetoothctl disconnect` cycles each reconnected in about 2 s with a fresh
  `0800` → echo → `0100` handshake. The capture contains exactly one `0800`, one
  echo, and one `0100` per session, with no extra retry writes. The virtual nodes
  stayed `event8`/`event18`. A trigger held across two disconnects emitted
  `BTN_SOUTH` release at each drop and re-pressed only because the physical
  button was still held. Backoff resets to 1 s after a session that streamed.
- **Settings (`05 00`).** 68 bytes: imu 5 ms, mag 10 ms, button mask `0x3f`,
  touch 320×320, firmware `QD4`, model `ET-YO324`, serial `RF7JB2ACG1D`.
- **Calibration (`03 00`).** 66 bytes. The last two bytes are `2500` = 25.00 °C,
  so the temperature is centi-degrees, not whole degrees. Bit 0 is the only bit
  that is zero in all eight groups, which confirms the LSB is the padding bit.
  Decoding the groups as two 3×3 matrices does not produce a plausible
  alignment (row norms 0.58–1.23 at 2^16, no diagonal dominance), so the
  semantic layout is still unresolved. This unit's decoded groups are pinned in
  the fixture test.

## Virtual DualSense validation (2026-10-20)

Built with `--features dualsense` and run as:

```sh
gearvr-cli run 'Gear VR' --backend dualsense
```

- The kernel bound `hid_playstation` and registered a DualSense:
  `Registered DualSense controller hw_version=0x01000208 fw_version=0x01000036`.
- Three input nodes appeared: `Wireless Controller`, `Wireless Controller
  Motion Sensors`, and `Wireless Controller Touchpad`.
- Trigger produced `BTN_TR2`, Home `BTN_MODE`, and Back `BTN_SELECT`. The motion
  node streamed live data, with observed ranges of about ±1 g and ±512 °/s while
  the controller was moved.
- Motion is remapped from the Gear VR frame (`+X` right, `+Y` toward the
  trigger, `+Z` up) into SDL's PlayStation frame (`+X` right, `+Y` up, `+Z`
  toward the player) as `(x, z, -y)`. Yaw, roll, and pitch directions were
  checked in Steam and all match.
- The raw gyro bias (about `(4, -51, 14)` counts at rest) caused a steady drift
  in Steam. Subtracting the fusion's stationary bias estimate removes it; the
  estimator needs about two seconds of stillness after connecting to initialize.
- Steam logged `Product: DualSense Wireless Controller` and the SDL mapping
  `54c/ce6 ... touchpad:b11,misc1:b12,crc:5657,platform:Linux`.
- The synthetic feature reports (`0x05` calibration, `0x09` pairing info, `0x20`
  firmware info) are answered with a CRC over seed `0xA3`, using the 279-byte
  Bluetooth report descriptor.
- Selecting `--backend evdev` creates the two `1209` nodes and no `054c` uhid
  device, so rollback is just omitting the flag.

The virtual identity is VID/PID `054c:0ce6`, name `Wireless Controller`, MAC
`02:47:56:52:00:01`, `BUS_BLUETOOTH`. It is opt-in and mutually exclusive with
the evdev gamepad.

Steam's desktop controller layout accepts the gyro binding, and yaw/roll/pitch
all move the cursor in the expected direction. Remaining: a long-session run and
packaging the service.

## Not yet validated

- Pairing-agent behavior, bonding persistence, reconnect after sleep, and
  connection-parameter requests are untested.
- Sleep/wake was exercised as ten forced disconnect/reconnect cycles and as one
  natural inactivity sleep (~3.5 minutes). Multi-hour sleep, wake-button
  behavior after a long rest, and recovery from a wedged adapter remain
  untested.
- Touchpad gestures (swipe/tap) are not synthesized; only raw position and
  click are exposed. The defaults remain off.
- Magnetometer scale and mounting rotation are still unverified. The `0300`
  calibration record is captured, but its semantic layout is unresolved.
- Volume up/down bits were not exercised.
- The virtual DualSense is enumerated by the kernel and Steam, and gyro works
  in a desktop layout. A long-session run and the systemd service remain.
- The systemd user unit is in `packaging/systemd/gearvr.service`, but has not
  been run on the Deck. It assumes the controller is already paired/trusted and
  restarts the process after a Bluetooth disconnect.
