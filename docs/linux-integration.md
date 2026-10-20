# Presenting the controller to Linux

The controller has no HID descriptor carrying its interesting data, so the driver
has to synthesize a device. There are three viable mechanisms, and the choice
determines what Steam and VR runtimes can do with it.

## Why there is no kernel driver

| Approach | Feasible? | Notes |
| --- | --- | --- |
| `hid-*` kernel driver | **No** | Requires a HID report descriptor containing the IMU/trigger/touchpad data. The `0x1879` service has a descriptor, but it only declares a keyboard, a 1-byte vendor report and a consumer-control report. No sensor data. |
| Kernel Bluetooth GATT client driver | Technically yes | Would live under `net/bluetooth` as a GATT client, parse the vendor service, and register an input device. Nothing like it exists in-tree; enormous effort to reimplement what already runs fine in userspace; needs privileged BLE handling and firmware-specific quirks in kernel code. |
| **Userspace driver (this project)** | **Yes** | Consumes BlueZ/D-Bus GATT, presents the result via `uinput` or `uhid`. This is what every existing project does. |

Note that `drivers/hid/hid-ovr.c`, which turns up when you search for a "Gear VR
HID driver", is for the **headset's USB tracking camera**, not the controller.
Linux 6.9's `hid-samsung` additions cover Samsung's *wireless gamepad* (USB product
`a000`), keyboards and mouse — all genuine HID-over-GATT devices, and none of them
is this one.

## Mechanism 1 — `uinput` (evdev gamepad)

The driver opens `/dev/uinput`, declares a gamepad with the axes and buttons we
choose, and writes `input_event` structs. The kernel creates
`/dev/input/eventN` plus a js node.

**Pros**

- Simplest to implement; `evdev`/`uinput` crates are mature.
- Universally consumable: SDL2, Steam, RetroArch, `jstest`, anything.
- No privileged BLE; only `/dev/uinput` access (a udev rule with `uaccess` or a
  `uinput` group).
- Survives driver crashes gracefully (node disappears, no wedged HID state).

**Cons**

- **No gyro.** evdev has no standard axis or usage for motion sensors on a
  gamepad. SDL won't see it; Steam Input won't either. Your 3-DOF data has
  nowhere to go except through fake axes.
- Device is generic — Steam treats it as an unknown pad (see the Steam caveats
  below).

**How to encode 3-DOF anyway:** the practical trick is to expose the *fused
orientation* as fake axes, e.g. `ABS_RZ`/`ABS_THROTTLE` for yaw/pitch deltas, and
let the consumer treat them as "mouse-like" motion. This is what the existing
mouse-oriented projects do (`REL_X`/`REL_Y` on a virtual mouse). It works but is
a convention, not a standard, so each consumer needs a mapping.

## Mechanism 2 — `uhid` (virtual HID device)

The driver writes `/dev/uhid` and the kernel instantiates a real HID device with
a report descriptor **we supply**. This is strictly more powerful: we can emulate
a device the host already understands deeply.

**Two sub-options:**

### 2a. Generic HID gamepad via `uhid`, `BUS_USB`

What `sameer/gearvr-controller-uhid` does (albeit for a mouse). Gives you a HID
device with a descriptor you control, so you can declare motion axes properly and
even a vendor collection for raw IMU.

- Pro: precise control over the descriptor; HID consumers can read raw reports
  via hidraw.
- Con: **SDL2's hidapi backend deliberately rejects `uhid` devices with
  `BUS_USB`** because hidapi looks for a parent USB device in the udev tree and
  finds none, then falls back to the generic `sysjoystick` driver, which lacks
  advanced features (this is documented in detail by the inputtino author). So a
  `BUS_USB` uhid device behaves much like a plain evdev pad for SDL/Steam, while
  *looking* more capable.

### 2b. Virtual **DualSense over `uhid` with `BUS_BLUETOOTH`** — the interesting option

This is the technique used by [inputtino](https://github.com/games-on-whales/inputtino)
(used by Sunshine / Wolf for game streaming) and written up in
"[Beyond USB: Improving Virtual Controller Support in Linux Games](https://abeltra.me/blog/inputtino-uhid-3/)":

- Create the uhid device with `BUS_BLUETOOTH` instead of `BUS_USB`.
- Supply the DualSense report descriptor.
- Append the **CRC32** checksum that DualSense requires on Bluetooth reports.
- Send the current state continuously (~every 16 ms) even when unchanged, because
  SDL times out during init if nothing arrives.

**Why this matters enormously for our use case:**

SDL2's hidapi path then *accepts* the device, meaning Steam sees a **real
DualSense** — a controller whose features Steam Input fully supports, including
**gyro, touchpad and battery**. The hidapi check that fails for `BUS_USB` (no
parent USB device in udev) is bypassed for `BUS_BLUETOOTH`, where hidapi just
takes the product string.

So the controller's raw 9-axis data can be fused by us and then emitted as
DualSense gyro reports, and Steam Input will treat it as native gyro input on a
supported pad. That is the only route to Steam-side gyro mapping without patching
Steam.

**Cons:** much more work (report format, CRC32, timing, staying faithful enough
that SDL's PS5 driver doesn't reject it), and you are impersonating a Sony device,
which is legally/ethically fine for interoperability in the EU but is worth a
note. Also: a virtual DualSense is *not* what a VR runtime wants.

### Implemented status (2026-10-20)

`gearvr-output` now has an opt-in `dualsense` backend built on `/dev/uhid`:

```sh
cargo build --release -p gearvr-cli --features dualsense
gearvr-cli run 'Gear VR' --backend dualsense
```

It creates `054c:0ce6` with `BUS_BLUETOOTH`, the name `Wireless Controller`, a
stable virtual MAC (`02:47:56:52:00:01`), and answers the kernel's feature
reports (`0x05` calibration, `0x09` pairing info, `0x20` firmware info). Reports
are sent every 16 ms, including when the state is unchanged, and a neutral report
is sent on disconnect. On the Deck the kernel registered the DualSense and Steam
generated the `54c/ce6` SDL mapping, including the touchpad.

Rollback is just omitting `--backend dualsense`; the evdev gamepad path is
unchanged and the uhid device is destroyed when the process exits. Do not run
both backends at once, or Steam will see two controllers.

## Mechanism 3 — direct API / OpenXR integration

Instead of presenting a generic device, the driver could act as an
**OpenXR device provider** (Monado plugin / SteamVR driver) so that VR apps get a
3-DOF controller pose directly.

- Monado has an `evdev` controller driver that reads *existing* gamepad devices and
  can add 3-DOF poses to them; a Gear VR controller exposed as a well-formed evdev
  pad with motion axes may or may not be picked up usefully.
- `jeong-jimin-github/psvr-steamvr` is an experimental SteamVR driver that
  integrates a Gear VR controller (Windows).
- This is the "correct" answer for VR and the wrong answer for desktop mode.

## Recommended architecture: layered outputs

Because one output format cannot serve both "Steam in desktop mode" and "VR app",
build the driver as **core + pluggable output backends**:

```
BLE (BlueZ) → packet parser → fusion (Madgwick/Mahony) → ┬→ uinput gamepad        (universal, no gyro)
                                                         ├→ uhid DualSense (BT)    (Steam Input w/ gyro)
                                                         └→ JSON/UDP/ZMQ/OpenXR    (VR runtimes, debug)
```

All backends active simultaneously is fine (they are independent virtual devices).
Ship `uinput` first because it gets you a working, testable driver in a day; add
the DualSense backend when you want gyro in Steam.

## Steam Input caveats you must design around

These are real and current, and they affect the whole plan:

1. **Steam Input only exposes gyro for devices it has a native mapping for** —
   Steam Controller, PS4/PS5 (DualSense/DualSense Edge), Switch Pro/Joy-Cons.
   Xbox pads have no gyro at all. A **generic evdev gamepad gets no gyro source in
   the configurator**, no matter what axes you expose. Official input-source
   documentation lists Gyro only for those three families.
   ⇒ *If you want Steam to map gyro-to-mouse, emulate a DualSense (mechanism 2b).*
   If you only need the touchpad/buttons mapped, plain evdev is fine.

2. **There is an open Steam-for-Linux regression (2026) where generic
   evdev/DirectInput gamepads are enumerated but never opened by Steam Input**
   after udev/systemd 259 on Fedora 44, with HIDAPI (`hidraw`) controllers
   unaffected. Reported on Fedora 44 and reproduced on Ubuntu 24.04. Reported
   working around 2026-06-23, broken from 2026-06-30 onward; still open as of
   October 2026. Workaround: disable Steam Input for that game, which loses
   remapping.
   ⇒ *Another strong argument for the DualSense/hidraw route, and worth verifying
   on your actual Steam Deck before committing to the evdev design.*

3. **Steam hides the physical device from games while providing a virtual one.**
   If Steam Input fails to activate (issue above), the controller is dead in every
   Steam-Input game. So a fallback path that does not go through Steam matters,
   and the driver should be able to run with Steam Input disabled.

4. Steam Deck's `steam-devices` udev rules and SDL version differ from desktop
   Fedora's, so test on the actual target device, not just on `dev`.

## Permissions and packaging

- `/dev/uinput`: needs write access. Standard approach is a udev rule granting
  `uaccess` or a dedicated group:
  `KERNEL=="uinput", MODE="0660", GROUP="uinput", OPTIONS+="static_node=uinput"`
- The optional motion node needs an explicit `uaccess` rule. It has only
  `EV_ABS` and `INPUT_PROP_ACCELEROMETER`, so udev does not classify it as a
  joystick. The rule **must sort before `71-seat.rules`**; use
  `packaging/udev/70-gearvr-evdev.rules`, not a `99-` rule. A late rule adds
  the tag after seat assignment and leaves the node inaccessible.
- `/dev/uhid`: same idea; needs `SUBSYSTEM=="misc", KERNEL=="uhid"` access.
- BlueZ: talking to `org.bluez` over the system D-Bus as a normal user is fine;
  *pairing* is easiest via `bluetoothctl` or an agent registered by the daemon.
  A driver that can pair itself needs a `org.bluez.Agent1` implementation.
- Consider a systemd **user** service so the driver starts on login and restarts
  after the controller sleeps.

## What was measured on this target

On Bazzite 44 with the synthetic evdev backend, Steam logged the generic
`1209:0001` pad, generated an SDL mapping, and loaded
`configset_controller_generic.vdf`. The worst-case Caveat 2 (Steam enumerates
but never opens a generic evdev pad) did not reproduce on this build. Caveat 1
is unchanged: the generic pad has no gyro, so Steam-side gyro still needs the
`uhid` DualSense route. Details are in [`implementation.md`](implementation.md).

## Rust crate choices

| Need | Crate | Notes |
| --- | --- | --- |
| BLE / GATT | `bluez-async` | Actively maintained, async, D-Bus. Preferred. |
| | `bluer` | Pure-Rust alternative, decent API, fewer BlueZ quirks handled for you. |
| | `btleplug` | Cross-platform (also Windows/macOS), but historically weak on BlueZ-specific quirks and connection parameter handling. |
| | `blurz` | What sameer's driver uses. Effectively unmaintained. Avoid. |
| uinput | `evdev` (has `uinput` support) | Mature, well-documented. |
| uhid | `uhid` / `uhid-fs` | `uhid-fs` is what sameer used; small API. |
| Sensor fusion | `nalgebra` + own Madgwick/Mahony, or port `gamepad-motion-helpers` (Rust port exists inside daydream-catcher) | Jibb Smart's algorithm is tuned for gamepad gyro and is the better choice for pointer use. |
| Async runtime | `tokio` | Works with `bluez-async`. |

## Reference implementations to study

- `sameer/gearvr-controller-uhid` — smallest end-to-end Rust + BlueZ + uhid
  example, though it has protocol bugs (see `prior-art.md`).
- `nullstalgia/daydream-catcher` — best Rust packet parser (`deku` bitfields) and
  a `uhid` air-mouse client.
- `games-on-whales/inputtino` — how to build a convincing virtual DualSense over
  uhid over Bluetooth, including the CRC32 and report timing details.
- `abeltra.me` posts on why SDL accepts `BUS_BLUETOOTH` uhid devices but not
  `BUS_USB` ones.
