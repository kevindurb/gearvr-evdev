# Prior art

Every project I could find that touches the Gear VR Controller's BLE protocol,
what each one is good for, and what it gets wrong. If you are writing a driver,
read this before writing code — several of these have subtle bugs that got copied
into downstream forks.

## Summary table

| Project | Lang | Platform | Output | Handshake correct? | Mag offset correct? | State |
| --- | --- | --- | --- | --- | --- | --- |
| [jsyang/gearvr-controller-webbluetooth](https://github.com/jsyang/gearvr-controller-webbluetooth) | JS | Web Bluetooth | 3D viewer demo | n/a (manual buttons) | **No** (reads offset 32) | 120★, 2018, reference only |
| [polygraphene/FreePIEVRController](https://github.com/polygraphene/FreePIEVRController) | C# | Windows (FreePIE) | virtual joystick | **No** (`08`+`01` back to back) | Yes | 2018 |
| [gb2111/Access-GearVR-Controller-from-PC](https://github.com/gb2111/Access-GearVR-Controller-from-PC) | C# | Windows | UI readout | **No** (only `01 00`) | Partial | 2019 |
| [sameer/gearvr-controller-uhid](https://github.com/sameer/gearvr-controller-uhid) | Rust | Linux (BlueZ + uhid) | uhid mouse | **No** (only `01 00`) | n/a (no mag use) | 14★, 2019, Apache-2.0 |
| [rdady/gear-vr-controller-linux](https://github.com/rdady/gear-vr-controller-linux) | Python | Linux (pygatt + uinput) | mouse / browser nav | **No** (only `01 00`) | n/a | 54★, 2018, "PoC" |
| [ilyabru/GearVrController4Windows](https://github.com/ilyabru/GearVrController4Windows) | C# | Windows | mouse | No | n/a | 2019 |
| [rdady/gear-vr-controller-windows](https://github.com/rdady/gear-vr-controller-windows) | C#/LINQPad | Windows | mouse | No | n/a | 2021 |
| [cwig/gearvr-exploration](https://github.com/cwig/gearvr-exploration) | JS | Web Bluetooth | 3D viewer + **mag cal** | n/a | **Yes** (offset 48) | 2021, best mag work |
| [uutzinger/gearVRC](https://github.com/uutzinger/gearVRC) | Python | Linux/Win (bleak) | AHRS fusion → ZMQ/serial/viewer | No (`-vr` documented broken) | Yes | 2023, most complete |
| [kaminoer/Dream-Mouse](https://github.com/kaminoer/Dream-Mouse) | C#/.NET | Win, Android, **Linux WIP** | air mouse + radial menu | Unknown | n/a | 45★, active 2026 |
| [nullstalgia/daydream-catcher](https://github.com/nullstalgia/daydream-catcher) | Rust | cross (no_std crates) | packet crates + uinput client | n/a (library) | n/a | 2025, best protocol reference |
| [dennisonbertram/gearvr-controller](https://github.com/dennisonbertram/gearvr-controller) | Python/Swift | macOS primarily | air mouse + tools | **Yes** (documents it) | Partial | 2025, best doc |
| [Tinnci/gear_vr_controller](https://github.com/Tinnci/gear_vr_controller) | Rust | Windows (WinUI3) | air mouse / presenter | Unknown | n/a | 2026, polished but Windows-only |
| [jeong-jimin-github/psvr-steamvr](https://github.com/jeong-jimin-github/psvr-steamvr) | C++ | Windows (SteamVR) | SteamVR driver integration | Unknown | n/a | experimental |
| [vladoshub/xr-gate](https://github.com/vladoshub/xr-gate) | C++ | ? | provider abstraction | Unknown | n/a | experimental |

## The three that actually matter for a Linux Rust driver

### 1. sameer/gearvr-controller-uhid — the closest architectural match

**What it is:** ~200 lines of Rust. Uses `blurz` (BlueZ D-Bus bindings) to find an
already-connected device advertising the vendor service, subscribes to `…d281`,
and writes a `uhid` device node so the kernel sees a real HID mouse.

**Why it matters:** it is the only existing project that produces a genuine HID
device rather than synthesizing input with `xdotool`/`uinput`-mouse. The uhid
approach is the right one for a mouse-shaped output.

**What it gets wrong:**

- Sends only `01 00` — runs at the slow rate (~30 pkt/s with gaps) instead of 68.
- Parses **only the first IMU sample** of each packet (samples 2 and 3 dropped).
  It reads `data[54..57]` for the axis correctly, but `Packet::from` takes
  `timestamp` from `data[0..4]` and ignores 16–47 entirely.
- Hardcodes a fake USB vendor/product (`0x15d9`, `0x0a37`) in the HID descriptor,
  so it shows up as a generic Saitake-ish mouse.
- Requires the device to be **already connected** — it filters on
  `is_connected()` and does no reconnect supervision.
- Uses `blurz`, which is effectively unmaintained; you would want `bluez-async`
  or `bluer` today.
- No trigger button mapping in the descriptor (`Buttons::Into<u8>` only maps
  touchpad and trigger to two bits, and the report descriptor declares 3 buttons
  but the code only sets 2).

**Verdict:** best starting skeleton in Rust, but you would rewrite the BLE layer
and the parser.

### 2. nullstalgia/daydream-catcher — the best protocol implementation

**What it is:** a workspace of `no_std`-compatible Rust crates. `daydream-catcher`
parses both Daydream and GearVR packets using `deku` bitfields and does all unit
conversions. `daydream-airmouse-uinput` is a Linux uinput client.

**Why it matters:** the packet parsing is the most rigorous available — proper
bitfield handling for the 10-bit touchpad packing, correct 21-bit calibration
record layout, correct magnetometer offset, and a detailed writeup of what each
command actually does from both APK decompilation *and* Wireshark captures.

**Limits:** it explicitly targets an air-mouse, not a gamepad; the author calls it
a "testbed, not intended for daily use". It does not do the `08 00` handshake
(author had not yet noticed the ordering issue at the time of the writeup, though
the writeup does note the two connection-parameter states).

**Verdict:** steal the parser design. Reuse the crate if `no_std` bitfields suit
you; otherwise port the bit layouts.

### 3. uutzinger/gearVRC — the best sensor fusion

**What it is:** ~2500 lines of Python. Connects with `bleak`, parses all fields,
runs AHRS fusion via the author's `pyIMU` package, estimates gyro bias online when
stationary, supports magnetometer calibration, implements a "virtual wheel"
gesture on the touchpad rim, and publishes over ZMQ/serial.

**Why it matters:** it is the only project that treats the IMU seriously. The
documented axis remapping (`acc −Y−X+Z`, `mag −X+Y+Z`, `gyr +Y+X−Z`), the
stationary-detection gyro bias estimator, and the calibration workflow are all
worth copying.

**What it gets wrong / is limited by:**

- Documents `-vr` (VR mode) as **"[does not work]"** — so it also runs slow.
- Requires a **one-line patch to `bleak`** because the controller does not send
  the confirmation reply `bleak` asserts on (`assert reply is not None` →
  `if reply is not None:`). This is a real quirk of this device, not a bug in the
  driver.
- The magnetometer is "unusable without calibration" and the code says so
  bluntly.
- Heavy dependency list (PyQt5, OpenGL, pyglet, pywavefront, zmq, msgpack, uvloop).
- Windows is "cumbersome" — must remove the device from the OS between sessions.

**Verdict:** the fusion math and the bias estimator are the parts to port to Rust
(`nalgebra` + a Madgwick/Mahony implementation, or the `gamepad-motion-helpers`
port inside daydream-catcher).

## Best documentation

### dennisonbertram/gearvr-controller (2025)

Verifies the protocol on real hardware and publishes `PROTOCOL.md` with:

- **the `08 00` → wait for ack → `01 00` handshake** and the evidence that every
  other project gets it wrong,
- hardware-verified axis convention (X right, Y forward, Z out of touchpad) tested
  against gravity in known poses,
- the button bit table including the idle flag,
- the `0500` device record including the real serial number,
- the observation that byte 59 is battery percentage,
- the touchpad lift marker semantics,
- the HID consumer channel on `2A4D`.

Its weakness: it is macOS-first. The Linux-side code is a Python script, not a
driver, and there is no evdev/uhid integration.

### nullstalgia's `Reference - GearVR (Gochugaru).md`

The most complete single page on GATT layout, including all nine SUOTA
characteristics with their individual UUIDs, the 40/68-byte settings record
layout, and the connection-parameter request behaviour (including that the
controller will **disconnect if it can't negotiate within 30 s**).

## Common mistakes found in the wild

Worth listing because they are all still present in forks:

1. **Reading the magnetometer at offset 32.** jsyang's original bug; offset 32 is
   sample 3's timestamp. Correct offset is 48. Present in jsyang's repo and in
   `MadTinker/gluetooth`, which copied it.
2. **Parsing only one IMU sample per packet.** Throws away 2/3 of the data and
   drops you from 206 Hz to 68 Hz.
3. **Sending `01 00` without the `08 00` handshake.** Caps you at ~30 pkt/s.
4. **Sending `08 00` and `01 00` back to back.** Kills the stream after ~1.5 s
   (polygraphene, and therefore anyone who ported the FreePIE plugin).
5. **Swapped volume up/down bits.** gb2111's C# port.
6. **Swapped or 8-bit touchpad axes.** gb2111 and at least one ESP32 attempt.
7. **Treating accel/gyro as 13-bit.** They are full i16.
8. **Assuming 60-byte packets only.** The controller sends 2, 12, 40, 60, 66 and
   68-byte packets depending on state.
9. **Not calling `trust` in BlueZ.** Bond exists but no auto-reconnect.
10. **Believing keep-alives prevent sleep.** They don't. Nothing does.

## Projects that are not what they look like

- **`hid-ovr.c`** (Samsung's Android kernel trees, e.g.
  `sm8150-samsung/android_kernel_samsung_r5q`): this is a **USB hidraw
  passthrough for the Gear VR headset's tracking camera**, vendor ID
  `Samsung Electronics`, product `GearVR_1`. It has nothing to do with the
  controller. Anyone searching for "gear vr hid driver" finds this and gets
  misled.
- **Linux 6.9's `hid-samsung` additions** add Samsung *wireless gamepad* (`a000`),
  wireless keyboards, action mouse, book cover, universal keyboard and HOGP
  keyboard. All are genuine HID-over-GATT peripherals with parseable report
  descriptors. The Gear VR Controller is **not** among them and cannot be.
- **`kozec/sc-controller` issue #399** (2018) asks for Gear VR Controller support
  in SC-Controller. It was never implemented; SC-Controller consumes evdev
  gamepads, and the controller does not present as one.
