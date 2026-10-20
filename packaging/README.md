# Packaging

## udev

Install `udev/70-gearvr-evdev.rules` as root. The `70-` prefix is deliberate:
`71-seat.rules` consumes the `uaccess` tag and `73-seat-late.rules` applies the
ACL. A `99-` rule is too late.

```sh
sudo install -m 0644 udev/70-gearvr-evdev.rules /etc/udev/rules.d/
sudo udevadm control --reload-rules
sudo udevadm trigger --subsystem-match=input
```

## Virtual DualSense (opt-in)

Build with the feature and select the backend:

```sh
cargo build --release -p gearvr-cli --features dualsense
gearvr-cli run 'Gear VR' --backend dualsense
```

This creates a Bluetooth DualSense (`054c:0ce6`, `Wireless Controller`) through
`/dev/uhid`. Steam sees a real DualSense and can use its gyro. The virtual MAC is
stable (`02:47:56:52:00:01`), so Steam keeps the same controller configuration.

Requirements and notes:

- Your user needs read/write access to `/dev/uhid`. On the Deck this is already
  granted; otherwise use a udev rule such as
  `KERNEL=="uhid", TAG+="uaccess"` (or the `input` group) and re-login.
- `hid_playstation` creates three nodes: the gamepad, motion sensors, and
  touchpad. Reading the motion node with `evtest` as a non-root user needs an
  additional ACL; Steam reads the HID device directly and does not need it.
- Do not run the DualSense backend and the evdev gamepad at the same time.
- Rollback is omitting the flag: `--backend evdev` remains the default and
  creates only the `1209` devices.

## systemd user service

Build/install `gearvr-cli` at `~/.local/bin/gearvr-cli` (or edit `ExecStart`),
pair the controller first, then:

```sh
mkdir -p ~/.config/systemd/user
cp systemd/gearvr.service ~/.config/systemd/user/
systemctl --user daemon-reload
systemctl --user enable --now gearvr.service
journalctl --user -u gearvr.service -f
```

The service restarts on failure. A Bluetooth disconnect currently ends the
session; `Restart=on-failure` reconnects a few seconds later. Pairing is a
desktop-agent job, not the driver's:

```sh
bluetoothctl
power on
agent on
default-agent
scan on
pair AA:BB:CC:DD:EE:FF
trust AA:BB:CC:DD:EE:FF
connect AA:BB:CC:DD:EE:FF
```

`trust` is important: without it BlueZ may refuse to reconnect after the
controller's activity-based sleep. A controller that wedges in a half-paired
state is usually fixed by `remove` + re-pair, not by deleting the driver's
state.
