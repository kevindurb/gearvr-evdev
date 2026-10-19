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
