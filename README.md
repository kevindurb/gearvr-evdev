# gearvr-evdev

A Linux userspace driver for the **Samsung Gear VR Controller (ET-YO324 / SM-GEARVR)**, written in Rust.

The controller is a 2017-era 3-DOF BLE motion remote that speaks a custom GATT
protocol rather than HID. This project reads that protocol and exposes the
controller as a **standard evdev gamepad**, so that:

- on a Steam Deck (or any Steam host) **Steam Input** can remap it to a
  mouse/keyboard for desktop mode, while
- VR applications can consume the same device as a normal 3-DOF controller.

No kernel module, no patched kernel, no vendor SDK.

## Status

**Pre-implementation.** This repository currently holds the research and design
documentation. See [`docs/DESIGN.md`](docs/DESIGN.md) for the intended
architecture and milestones.

## Documentation

Start at [`docs/index.md`](docs/index.md).

| Doc | Contents |
| --- | --- |
| [`docs/hardware.md`](docs/hardware.md) | Teardown, silicon, sensors, power, debug access, firmware/SUOTA |
| [`docs/protocol.md`](docs/protocol.md) | Advertising, GATT table, commands, connection parameters, pairing failures |
| [`docs/packets.md`](docs/packets.md) | Byte-level packet formats: IMU, touchpad, buttons, settings, calibration |
| [`docs/prior-art.md`](docs/prior-art.md) | Every existing driver/RE project, what each one got right and wrong |
| [`docs/linux-integration.md`](docs/linux-integration.md) | uinput vs uhid vs kernel driver; why there is no in-tree driver |
| [`docs/open-questions.md`](docs/open-questions.md) | Undecoded calibration blob, axis conventions, conflicting claims |
| [`docs/references.md`](docs/references.md) | Source list |
| [`docs/DESIGN.md`](docs/DESIGN.md) | What this project will build and how |

## License

MIT — see [`LICENSE`](LICENSE).
this project is a clean reimplementation from protocol documentation, not a
derivative of that code.
