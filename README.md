# gearvr-evdev

A Linux userspace driver for the **Samsung Gear VR Controller (ET-YO324, sold with the SM-R324 headset)**, written in Rust.

The controller is a 2017-era 3-DOF BLE motion remote that speaks a custom GATT
protocol rather than HID. This project reads that protocol and exposes the
controller as a **standard evdev gamepad**, so that:

- on a Steam Deck (or any Steam host) **Steam Input** can remap it to a
  mouse/keyboard for desktop mode, while
- VR applications can consume the same device as a normal 3-DOF controller.

No kernel module, no patched kernel, no vendor SDK.

## Status

**Working skeleton, validated without the physical controller.** The protocol
parser, 6-DOF fusion, command sequencer, evdev/JSON/UDP outputs, and CLI are
implemented. On a Steam Deck, a synthetic uinput gamepad was enumerated by udev,
read by `evtest`/`jstest`, and accepted by Steam Input as a generic controller.

The live BlueZ transport (`scan`, `doctor`, `run`) is implemented but has not
been exercised against a real ET-YO324 yet. The controller is required to settle
the axis convention, pairing/reconnect behavior, and actual handshake timing.

See [`docs/implementation.md`](docs/implementation.md) for what is built and
what the Deck validation actually showed, and
[`packaging/README.md`](packaging/README.md) for udev, pairing, and the systemd
user unit.

## Quick start

```sh
cargo test --workspace
cargo run -p gearvr-cli -- synth --duration 20 --backend evdev --motion
cargo run -p gearvr-cli -- decode --hex '0800'
cargo run -p gearvr-cli -- doctor
cargo run -p gearvr-cli -- scan --seconds 6
cargo run -p gearvr-cli -- run 'Gear VR'
```

`synth` is hardware-free. `run` expects a controller already paired in BlueZ
(or `--pair`, which needs a working pairing agent). The JSON backend writes one
object per packet to stdout; logs go to stderr.

## Spec-driven development

This repository uses [OpenSpec](https://openspec.dev). Project rules live in
`openspec/config.yaml`; planned work lives under `openspec/changes/` and merges
into `openspec/specs/` when archived. The OpenCode integration is committed in
`.opencode/`.

```sh
/opsx-propose "add a feature"   # in OpenCode: create a change with proposal, specs, design, tasks
/opsx-apply                     # implement the current change
/opsx-archive                   # merge the change into the main specs
openspec list                   # CLI view of active changes
openspec validate --all         # validate specs and changes
```

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
| [`docs/implementation.md`](docs/implementation.md) | Current implementation, Deck validation, and remaining gaps |

## License

MIT — see [`LICENSE`](LICENSE).
this project is a clean reimplementation from protocol documentation, not a
derivative of that code.
