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
| `gearvr-output` | Implemented | evdev/uinput gamepad, optional motion node, newline JSON, JSON/UDP. |
| `gearvr-cli` | Implemented | `synth`, `decode`, `info`, `scan`, `doctor`, and a live `run` loop. |

`cargo test --workspace` currently runs 86 tests: 33 protocol unit tests, 4
protocol property tests, 14 fusion, 19 BLE, 8 output, 4 core, and 4 CLI.
`cargo clippy --workspace --all-targets -- -D warnings` is clean.

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

## Not yet validated

- No physical ET-YO324 is available yet. The `bluez` transport compiles and the
  `scan`/`doctor` commands run, but connect/notify/write has not been exercised
  against a controller.
- Pairing-agent behavior, bonding persistence, reconnect after sleep, and
  connection-parameter requests are untested.
- Axis convention and sign are still not settled; the axis map is configurable,
  but the default is a guess from the packet docs.
- Touchpad gestures (swipe/tap) are not synthesized; only raw position and
  click are exposed.
- Magnetometer calibration and the `0300` calibration blob remain undecoded.
- A `uhid` DualSense backend does not exist yet. It is still the only route to
  Steam-side gyro.
- The systemd user unit is in `packaging/systemd/gearvr.service`, but has not
  been run on the Deck. It assumes the controller is already paired/trusted and
  restarts the process after a Bluetooth disconnect.
