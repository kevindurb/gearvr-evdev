# What we are building

A **Rust userspace driver** for the Samsung Gear VR Controller on Linux that
presents the controller as a virtual **evdev gamepad**, so the same physical device
can serve two purposes:

1. **Desktop mode on a Steam Deck** — Steam Input maps the virtual gamepad's
   touchpad, trigger and buttons onto mouse/keyboard actions.
2. **VR** — a VR runtime or game consumes the same device as a normal 3-DOF
   controller with buttons and an analog touchpad.

No kernel module. No patched Steam. No vendor SDK.

---

## Goals

| # | Goal |
| --- | --- |
| G1 | Connect to the controller over BLE reliably, including after sleep/wake. |
| G2 | Run at the **fast** rate (68 pkt/s, ~206 Hz IMU) by doing the `0800`→ack→`0100` handshake correctly. |
| G3 | Present a **stable, well-formed evdev gamepad** that SDL2/Steam recognise. |
| G4 | Expose touchpad, trigger, Home, Back, volume, battery and 3-DOF motion. |
| G5 | Be a good citizen: user-level service, no root, clean shutdown, no wedged BLE state. |
| G6 | Be testable without hardware (packet fixtures, parser unit tests). |

## Non-goals (for now)

- 6-DOF / positional tracking. The hardware is 3-DOF only.
- Rewriting firmware via SUOTA (documented in `hardware.md`, deliberately not done).
- Windows or macOS support.
- A kernel driver.
- Haptics (the controller has none).

---

## Why a gamepad, and why that is the right call

The existing projects all emit a **mouse**. That is wrong for this device if you
also want VR, because a mouse cannot drive a VR app. A gamepad is the only device
class that both Steam Input and VR runtimes understand.

**But there are three real caveats you should know before writing code.** Two of
them are new information and they affect the architecture.

### Caveat 1 — Steam Input has no gyro source for generic gamepads

Per Valve's official input-source documentation, **Gyro** as an input source
exists only for the Steam Controller, PS4 controllers and DualSense. Xbox pads
have no gyro; generic evdev pads have no gyro source in the configurator.

That means: **a plain evdev gamepad gives Steam your touchpad, trigger and buttons,
but it cannot map gyro-to-mouse.** Our motion data would be visible to the game
(if the game reads raw axes) but invisible to Steam Input.

Two ways to handle this:

- **Accept it.** Map the touchpad to mouse in Steam Input. Use the fused
  orientation only for VR. This is the simple path and probably fine for desktop
  mode — a touchpad-driven cursor works well.
- **Emulate a DualSense over `uhid` instead** (see `linux-integration.md`
  mechanism 2b). If the virtual device is created with `BUS_BLUETOOTH` and a
  DualSense report descriptor, SDL2's hidapi path accepts it and Steam sees a
  real DualSense — **with working gyro, touchpad and battery**. We fuse the Gear
  VR controller's IMU ourselves and emit it as DualSense gyro reports. This is how
  Sunshine/Wolf (inputtino) does it, and it is the only route to Steam-side gyro
  without patching Steam.

### Caveat 2 — there is a current Steam Input regression for generic evdev pads

Open Steam-for-Linux issue #13394: after a udev/systemd 259 update, **generic
evdev/DirectInput gamepads are enumerated by Steam but never opened by Steam
Input**, so no virtual controller is created and the pad is unusable in
Steam-Input games. Reported on Fedora 44 (2026-06-30 onset, still open as of
October 2026) and reproduced on Ubuntu 24.04. **HIDAPI (`hidraw`) controllers are
unaffected.**

If this is still live on your SteamOS build, a `uinput` evdev pad will enumerate
and then sit there doing nothing, with Steam hiding the physical device from games
while providing nothing in its place.

⇒ **Verify this on the actual Deck before committing.** It is the single largest
risk to the gamepad plan. If it bites, the DualSense route (Caveat 1's second
option) sidesteps it because it uses hidraw.

### Caveat 3 — one output cannot serve both worlds perfectly

A virtual DualSense is ideal for Steam and wrong for VR (a VR app wants a 3-DOF
motion controller, not a PS5 pad). A plain evdev pad is ideal for VR and gyro-blind
for Steam. So:

**Build the driver as a core + pluggable output backends.** That is the design
below. Ship the evdev backend first (a day's work, immediately testable), add the
DualSense backend when you want gyro in Steam.

---

## Architecture

```
                    ┌──────────────────────────┐
   BLE (BlueZ)  ──► │ ble/  connection manager │  bond handling, reconnect,
   D-Bus            │      command sequencer   │  keep-alive, sleep detection
                    └────────────┬─────────────┘
                                 │ raw notifications
                    ┌────────────▼─────────────┐
                    │ protocol/  packet parser │  length-dispatched,
                    │  (zero-copy, no I/O)     │  100% unit-testable
                    └────────────┬─────────────┘
                                 │ SensorPacket { samples[3], suffix }
                    ┌────────────▼─────────────┐
                    │ fusion/  AHRS + bias est │  Madgwick/Mahony, gyro bias,
                    │  + mag calibration       │  −30° mag board correction
                    └────────────┬─────────────┘
                                 │ ControllerState
                    ┌────────────▼─────────────┐
                    │ output/  (trait Backend) │
                    └──┬─────────┬──────────┬──┘
                       │         │          │
                 evdev/uinput  uhid      json/udp
                  gamepad     DualSense   debug + VR bridge
```

### Layering rules

- `protocol/` has **no I/O and no async**. Pure functions over byte slices.
  This is where the tests live.
- `fusion/` is pure math over `protocol` types. Also testable with synthetic
  traces.
- `ble/` owns all async, D-Bus, and device quirks.
- `output/` backends implement a single `Backend` trait:
  ```rust
  trait Backend {
      fn on_state(&mut self, state: &ControllerState) -> Result<()>;
      fn on_connect(&mut self) -> Result<()>;
      fn on_disconnect(&mut self) -> Result<()>;
  }
  ```

### Suggested crate layout

```
gearvr-evdev/
├── Cargo.toml            # workspace
├── crates/
│   ├── gearvr-protocol/  # no_std-friendly parser + packet types  ← start here
│   ├── gearvr-fusion/    # AHRS, bias estimation, calibration
│   ├── gearvr-ble/       # bluez-async connection manager, command sequencer
│   ├── gearvr-output/    # Backend trait + evdev + uhid-dualsense + json
│   └── gearvr-cli/       # binary: config, CLI, service entrypoint
└── docs/
```

Keeping the parser in its own crate is worth it: it compiles fast, has no
dependencies, and can be fuzzed and tested in isolation. It also means the parser
can be reused by an ESP32 port later (feasible — BLE client on ESP32-S3 exists,
and there is prior art: `daydream-airmouse-esp`).

---

## The command sequencer (the thing that must be right)

This is the part almost every existing driver gets wrong. Implement it as an
explicit state machine, not as a sequence of `await`s.

```
States: Disconnected → Connected → Negotiating → Streaming → LowPower → …
```

On connect, to reach fast streaming:

1. Ensure bonded. If any characteristic read fails with an encryption error,
   trigger bonding (this is one reliable way to force a pair).
2. If not trusted/bonded, surface a clear error telling the user to run
   `bluetoothctl pair/trust <MAC>`. Do not silently fail.
3. Write `08 00` (VR mode).
4. **Wait for the `08 00` echo on the data characteristic**, up to ~3 s
   (expected ~1.5 s). Do **not** write `01 00` before this arrives.
5. Write `01 00` (sensor mode).
6. Verify packets are arriving; if the rate collapses or the stream stops, back
   off and retry from step 3 once, then fall back to plain `01 00` (slow mode)
   and log a warning.
7. Send `04 00` keep-alive every ~60 s. It will not prevent sleep — that is
   expected. It exists because some hosts/firmware behave slightly better with it.

On disconnect / sleep: return to `Disconnected`, wait for the controller to
reappear (it auto-reconnects to the bonded host), and restart from step 1. Do not
hammer it — back off exponentially.

Every command write must be idempotent and cancellable, and the state machine must
never leave a command half-written.

---

## Output backend 1 — evdev gamepad (build this first)

### What to expose

| Input | evdev mapping | Rationale |
| --- | --- | --- |
| Touchpad X/Y | `ABS_X`, `ABS_Y` (0–315) | Analog absolute position; Steam treats it as a stick or trackpad-ish source |
| Touchpad click | `BTN_A` (or dedicated) | |
| Trigger | `BTN_TRIGGER` / `BTN_TR` | Analog? No — the trigger is a **binary** switch, not analog. Map to a button. |
| Home | `BTN_MODE` | Conventionally the "guide" button |
| Back | `BTN_SELECT` | |
| Volume up/down | `BTN_TL` / `BTN_TR`? | Prefer **not** to expose, or expose as `KEY_VOLUMEUP/DOWN` on a separate node — they are system keys, not gamepad inputs, and mapping them as buttons confuses Steam. Default: **suppress**, optional flag to emit. |
| Fused orientation | `ABS_RX`, `ABS_RY`, `ABS_RZ` as **relative-ish deltas**, or a separate node | evdev has no motion-sensor class. Two options: (a) reuse right-stick axes for yaw/pitch, (b) expose gyro as `ABS_RX/RY/RZ` in mrad/s on a **second** input node so VR/curiosity consumers can read it without polluting the pad. **Recommend (b).** |
| Battery | not evdev | Read via `0x180F`; expose over the JSON backend and/or `UPower`-style file, not as an input axis |

### Design decisions

- **Two nodes** by default: a gamepad (buttons + touchpad) and a motion node
  (gyro as `ABS_RX/RY/RZ`, and accel as `ABS_X/Y/Z` on that second node). This
  keeps the pad clean for Steam while giving VR/analysis code real numbers.
- Set `ABS_X/ABS_Y` ranges to 0–320 with a small deadzone at the edges, matching
  the device's reported full scale.
- Use `input_id` with a distinctive vendor/product (or a documented fake pair) and
  a clear name like `Gear VR Controller (virtual)` so udev rules and Steam configs
  can target it deterministically.
- Emit `SYN_REPORT` after each state change; coalesce to ~120 Hz max to avoid
  flooding.

### Steam Input on top of this

- The virtual pad appears as a generic gamepad. Map touchpad → mouse in Steam
  Input's controller layout.
- **Do not** expose the gyro as a fake stick and expect Steam to treat it as gyro.
  It will not.

---

## Output backend 2 — virtual DualSense over uhid (phase 2, for gyro in Steam)

Only if Caveat 1 matters to you. Implementation notes are in
`linux-integration.md`; the essentials:

- `uhid` create with `BUS_BLUETOOTH`.
- DualSense report descriptor (reuse the well-known one).
- **CRC32** appended to each Bluetooth report (DualSense requires it).
- Send state **every ~16 ms continuously**, even unchanged, or SDL times out.
- Map: touchpad → DualSense touchpad, trigger → R2, Home → PS button,
  Back → Create, volume → suppress, fused orientation → gyro/accel fields.
- Expect to spend real time getting SDL's `SDL_hidapi_ps5.c` to accept it.

---

## Output backend 3 — JSON/UDP (start this on day one anyway)

A trivial backend that dumps decoded state as JSON lines or UDP datagrams. It costs
an hour, and it is how you will:

- debug without fighting Steam,
- write integration tests against recorded traces,
- let a VR runtime or a research script consume the data without any HID games,
- compare our fusion against `gearVRC`'s output on the same hardware.

---

## Milestones

| M | Deliverable | Done when |
| --- | --- | --- |
| **M0** | `gearvr-protocol` parser + fixtures | Unit tests pass against recorded 60-byte packets; handles 2/12/40/60/66/68-byte lengths; zero-sample packets don't panic |
| **M1** | JSON backend + CLI scan/connect | `gearvr-cli monitor` prints live buttons/touchpad/IMU on real hardware at **68 pkt/s** |
| **M2** | Command sequencer + reconnect | Survives sleep/wake 10× in a row without wedging; recovers from a mid-stream disconnect |
| **M3** | evdev gamepad backend | `jstest` / `evtest` shows sane values; Steam sees the pad; RetroArch works |
| **M4** | Fusion + motion node | Orientation tracks reasonably; gyro bias compensated; documented axis convention verified against gravity |
| **M5** | Steam Deck validation | On actual hardware: Steam Input layout works in desktop mode. **This is the gate that tells us whether M6 is needed.** |
| **M6** | (conditional) DualSense backend | Steam Input shows a DualSense with working gyro |
| **M7** | systemd user service + packaging | Installs without root; starts on login; logs to journal |

Ship M0–M3 before touching fusion. A driver that presents a correct gamepad at the
right packet rate is already better than every existing project.

---

## Testing strategy

- **Recorded fixtures.** Capture real notification bytes to files (the JSON backend
  makes this easy) and check them into `crates/gearvr-protocol/tests/fixtures/`.
  Parse-and-assert on the decoded values.
- **Synthetic traces.** Generate ideal accel/gyro traces for known rotations and
  assert the fusion output. This catches axis-convention and scale errors without
  hardware.
- **Property tests.** Arbitrary byte slices must never panic the parser. Use
  `proptest` or `arbitrary` + `cargo-fuzz`. The parser takes hostile-length input
  from a BLE device; treat it as untrusted.
- **Hardware-in-the-loop script.** A "pose test": prompt the user to hold the
  controller in N known poses, assert gravity direction and gyro signs. This is
  how we resolve `open-questions.md` #5 definitively.
- **Manual checklist** for the things automation cannot cover: pairing from
  scratch, wedged-pairing recovery, Steam behaviour on the Deck.

## Risks

| Risk | Mitigation |
| --- | --- |
| Steam Input ignores generic evdev pads (open regression) | Verify on the Deck at M5; fall back to DualSense backend |
| Steam cannot see gyro on a generic pad | Accept (touchpad-only mapping) or DualSense backend |
| Controller sleeps mid-session | Reconnect supervisor; document that nothing prevents sleep |
| Pairing wedges | Provide a `gearvr-cli doctor` command that detects the wedged state and prints exact `bluetoothctl` recovery steps |
| Axis convention is wrong | Resolve at M4 with the pose test, before building on it |
| `bleak`-style MTU/reply quirks | We control the BLE stack choices (`bluez-async`/`bluer`); test on Fedora 44 + SteamOS |
| Controller firmware variance (QC1/QD4/QBG) | Log firmware string from `0500`; treat settings fields defensively |

## Licensing note

Prior art differs: `sameer/gearvr-controller-uhid` is Apache-2.0,
`jsyang` and `cwig` are GPL-3.0, `gearVRC`'s status is unclear, `daydream-catcher`
and `dennisonbertram` vary. **This project should be a clean-room
reimplementation from the published protocol documentation**, not a port of any
GPL code. Facts (byte offsets, UUIDs, scaling factors) are not copyrightable;
code is. Cite the docs, write your own code, and the license can be whatever you
want (MIT/Apache-2.0 dual is the Rust default and a good choice).
