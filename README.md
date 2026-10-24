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

**Validated against a physical ET-YO324 on a Steam Deck.** Pairing, the
`0800` → echo → `0100` handshake, ~67 packets/s streaming, buttons, touchpad,
raw motion, the settings record, and the 66-byte calibration record all work on
a retail unit. Two hardware findings are recorded in
[`docs/implementation.md`](docs/implementation.md): a transposed service UUID
(now fixed) and the calibration record's still-unresolved semantic layout.

Still open: gestures and multi-hour sleep behavior. The magnetometer works with
an opt-in per-unit calibration profile.

See [`docs/implementation.md`](docs/implementation.md) for what is built and
what the Deck validation actually showed, and
[`packaging/README.md`](packaging/README.md) for udev, pairing, and the systemd
user unit.

## Install from a release

Published releases provide an x86_64 tarball and an RPM. Both target glibc 2.35
or newer and link the system D-Bus client library; the packaged binary includes
the optional DualSense backend.

```sh
# Fedora / bootc
sudo dnf install ./gearvr-evdev-0.1.0-1.x86_64.rpm
systemctl --user enable --now gearvr.service

# Any supported distribution, from the tarball
tar -xzf gearvr-evdev-0.1.0-x86_64-linux-gnu.tar.gz
cd gearvr-evdev-0.1.0
./install.sh --user
sudo ./install.sh --udev
```

The RPM does not enable the service automatically. See
[`packaging/README.md`](packaging/README.md) for the bootc Containerfile
snippet, multi-user caveats, permissions, pairing, and removal. In a release
archive that guide is included as `INSTALL.md`.

## Quick start

```sh
cargo test --workspace
cargo run -p gearvr-cli -- synth --duration 20 --backend evdev --motion
cargo run -p gearvr-cli -- decode --hex '0800'
cargo run -p gearvr-cli -- doctor
cargo run -p gearvr-cli -- scan --seconds 6
cargo run -p gearvr-cli -- capture 'Gear VR' --seconds 60 -o capture.jsonl
cargo run -p gearvr-cli -- probe 'Gear VR' --commands 03,05 -o records.jsonl
cargo run -p gearvr-cli -- replay capture.jsonl --include-tx --summary
cargo run -p gearvr-cli -- run 'Gear VR'
```

`synth` is hardware-free. `capture` records raw tx/rx traffic without creating
an input device; `probe` reads the constant settings/calibration records;
`replay` decodes a capture offline. `run` expects a controller already paired in
BlueZ (or `--pair`, which needs a working pairing agent). The JSON backend writes
one object per packet to stdout; logs go to stderr.

### Build modes

Local source builds use the vendored D-Bus client library, so they do not need
D-Bus development headers:

```sh
cargo build --release -p gearvr-cli
cargo build --release -p gearvr-cli --features dualsense
```

The release artifacts link the distribution's D-Bus client instead. That build
needs `pkg-config` and `libdbus-1-dev` (`dbus-devel` on Fedora):

```sh
cargo build --release --locked -p gearvr-cli \
    --no-default-features --features bluez,dualsense
```

`scripts/build-tarball.sh` runs the release build and checks the glibc and
`libdbus` baseline. The packaging guide has the container command and the full
release procedure.

Bringing up the physical controller for the first time? Follow
[`docs/bringup.md`](docs/bringup.md).

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
| [`docs/calibration.md`](docs/calibration.md) | Magnetometer capture, fit, and MARG behavior |
| [`docs/DESIGN.md`](docs/DESIGN.md) | What this project will build and how |
| [`docs/bringup.md`](docs/bringup.md) | First-contact checklist for a physical controller |
| [`docs/implementation.md`](docs/implementation.md) | Current implementation, Deck validation, and remaining gaps |

## License

MIT — see [`LICENSE`](LICENSE).
this project is a clean reimplementation from protocol documentation, not a
derivative of that code.
