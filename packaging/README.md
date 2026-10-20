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
- Steam's default desktop layout does not bind the DualSense touchpad click. In
  **Settings → Controller → Desktop Layout → Edit Layout**, select the DualSense
  and bind **Touchpad Click → Left Mouse Click**.
- Rollback is omitting the flag: `--backend evdev` remains the default and
  creates only the `1209` devices.

## systemd user service

Build and install the binary, pair the controller first, then:

```sh
cargo build --release -p gearvr-cli --features dualsense
mkdir -p ~/.local/bin ~/.config/systemd/user
cp target/release/gearvr-cli ~/.local/bin/
cp packaging/systemd/gearvr.service ~/.config/systemd/user/
systemctl --user daemon-reload
systemd-analyze --user verify ~/.config/systemd/user/gearvr.service
systemctl --user enable --now gearvr.service
journalctl --user -u gearvr.service -f
```

The unit runs `run --backend evdev --motion` from
`~/.local/bin/gearvr-cli`. `enable` makes it start with the graphical login
(user `default.target`). On a headless host, also run
`loginctl enable-linger "$USER"`.

The session supervisor owns reconnect, so a controller sleep or link drop no
longer ends the service: it re-runs the handshake with bounded backoff.
`Restart=on-failure` is only a safety net for a crashed process. Test both:

```sh
systemctl --user kill -s KILL gearvr.service   # systemd restarts it
systemctl --user stop gearvr.service           # clean stop, virtual devices removed
```

### DualSense instead of evdev

Use a drop-in so the packaged unit stays untouched:

```sh
mkdir -p ~/.config/systemd/user/gearvr.service.d
cat > ~/.config/systemd/user/gearvr.service.d/dualsense.conf <<'EOF'
[Service]
ExecStart=
ExecStart=%h/.local/bin/gearvr-cli run --backend dualsense
EOF
systemctl --user daemon-reload
systemctl --user restart gearvr.service
```

Verify with:

```sh
systemctl --user show -p ExecStart --value gearvr.service
ls /sys/bus/hid/devices/ | grep 054C
```

## Pairing

Pairing is a desktop-agent job, not the driver's:

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
