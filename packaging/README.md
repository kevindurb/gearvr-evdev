# Packaging and installation

Release artifacts are a tarball and a prebuilt-binary RPM built from the same
x86_64 Linux binary. The binary targets **glibc 2.35 or newer** and links the
system `libdbus-1.so.3`; install the distribution's D-Bus client library if it
is missing (`dbus-libs` on Fedora, `libdbus-1-3` on Debian/Ubuntu). RHEL 9 and
older glibc hosts are outside the supported baseline.

Both artifacts contain the driver binary, the systemd user unit, the udev
rules, and the license. The RPM installs them under `/usr`; the tarball installs
the user parts under your home directory and keeps the privileged udev step
separate.

## Build modes

Local source builds use the vendored D-Bus client library and need no D-Bus
development headers:

```sh
cargo build --release -p gearvr-cli
cargo build --release -p gearvr-cli --features dualsense
```

The release artifacts use the distribution's D-Bus client library instead, so
the binary does not bundle a C library. That build needs `pkg-config` and the
D-Bus development headers (`libdbus-1-dev` on Debian/Ubuntu, `dbus-devel` on
Fedora):

```sh
cargo build --release --locked -p gearvr-cli \
    --no-default-features --features bluez,dualsense
```

`scripts/build-tarball.sh` runs that build, stages the archive, and fails if the
binary does not link `libdbus-1.so.3` or requires a glibc newer than 2.35. Run it
inside an Ubuntu 22.04 container for the release baseline:

```sh
podman run --rm -v "$PWD:/src:Z" -w /src ubuntu:22.04 bash -lc '
  apt-get update &&
  apt-get install -y --no-install-recommends ca-certificates curl \
      pkg-config libdbus-1-dev build-essential binutils &&
  curl --proto "=https" --tlsv1.2 -sSf https://sh.rustup.rs \
      | sh -s -- -y --profile minimal --default-toolchain 1.85.0 &&
  . "$HOME/.cargo/env" &&
  ./scripts/build-tarball.sh
'
```

The archive is written to `dist/gearvr-evdev-<version>-x86_64-linux-gnu.tar.gz`.

## Tarball install

The archive contains a user installer, `install.sh`. It does not use root for
the user service, and it refuses to run as root in `--user` mode.

```sh
tar -xzf gearvr-evdev-0.1.0-x86_64-linux-gnu.tar.gz
cd gearvr-evdev-0.1.0
./install.sh --user
```

This installs the binary to `~/.local/bin/gearvr-cli` and the unit to
`$XDG_CONFIG_HOME/systemd/user` (normally `~/.config/systemd/user`), runs
`systemctl --user daemon-reload`, and enables and starts `gearvr.service`. If no
user manager is reachable (for example over SSH without lingering), it prints
the two commands to run after login. Running the installer again is safe and
replaces the same files.

Install the udev rules as root:

```sh
sudo ./install.sh --udev
```

That writes the rules to `/etc/udev/rules.d/`, reloads udev, and triggers the
input and `uinput`/`uhid` devices. The driver now has access to the device nodes
it needs, and the logged-in user can open the virtual gamepad and motion nodes.

Uninstall the user parts, then remove the rules if you no longer want them:

```sh
./install.sh --uninstall
sudo rm -f /etc/udev/rules.d/60-gearvr-permissions.rules \
             /etc/udev/rules.d/70-gearvr-evdev.rules
sudo udevadm control --reload-rules
```

`--uninstall` removes the user binary, unit, and enablement links. Captures,
calibration profiles, and other files under `~/.local/share/` are preserved.

## RPM install

The RPM packages the same binary for Fedora and bootc images. It does **not**
enable the user service; enable it explicitly:

```sh
sudo dnf install ./gearvr-evdev-0.1.0-1.x86_64.rpm
systemctl --user enable --now gearvr.service
journalctl --user -u gearvr.service -f
```

Installed paths:

| Path | Contents |
| --- | --- |
| `/usr/bin/gearvr-cli` | Driver binary |
| `/usr/lib/systemd/user/gearvr.service` | User service unit |
| `/usr/lib/udev/rules.d/60-gearvr-permissions.rules` | `uinput`/`uhid` access |
| `/usr/lib/udev/rules.d/70-gearvr-evdev.rules` | Virtual input node access |

Remove it with:

```sh
systemctl --user disable --now gearvr.service
sudo dnf remove gearvr-evdev
```

User captures and calibration profiles are not touched. If you also want to
remove the rules on a host where they were installed by hand, use the paths in
the table above.

## bootc images

Install the RPM at image build time, then enable the service globally:

```dockerfile
FROM ghcr.io/ublue-os/bazzite:stable

COPY gearvr-evdev-0.1.0-1.x86_64.rpm /tmp/
RUN dnf install -y /tmp/gearvr-evdev-0.1.0-1.x86_64.rpm \
 && dnf clean all

# Starts gearvr.service for every interactive user at login.
RUN systemctl --global enable gearvr.service
```

`systemctl --global enable` works offline in the image build and creates
`/etc/systemd/user/default.target.wants/gearvr.service`. The service starts when
the user manager starts at login; it is not a system service and does not run
before a login.

Global enablement assumes **one interactive user**, which is the normal gaming
image case. On a multi-user image, omit the global enable and have each user run
`systemctl --user enable --now gearvr.service`; otherwise every user's service
tries to connect to the same controller. For a headless host that needs the
service without a graphical login, also enable lingering with
`loginctl enable-linger "$USER"`.

If you prefer not to carry an RPM in the build context, copy the installed
files from an extracted tarball instead. The RPM is recommended because it
keeps the file list and removal path consistent.

## Migration from a manual install

The manual install used `~/.local/bin/gearvr-cli` and
`~/.config/systemd/user/gearvr.service`. The RPM uses `/usr`, so both can exist.
A user unit in `~/.config` takes precedence over the packaged unit in
`/usr/lib`, which can make it look like the RPM installed the wrong binary.

Before switching, check what is active and remove the old copy:

```sh
systemctl --user cat gearvr.service
command -v gearvr-cli
```

If the unit points at `%h/.local/bin/gearvr-cli`, stop and uninstall the manual
install first:

```sh
systemctl --user disable --now gearvr.service
rm -f ~/.config/systemd/user/gearvr.service ~/.local/bin/gearvr-cli
systemctl --user daemon-reload
```

Then install the RPM or run the tarball installer. Do not enable both paths at
once.

## Device permissions and security

`60-gearvr-permissions.rules` grants `uaccess` on `/dev/uinput` and `/dev/uhid`,
so the logged-in user can create the virtual devices. The output nodes
(`1209:0001`, `1209:0002`, and the `1209:0003` volume keyboard) are separately
tagged with `uaccess` by `70-gearvr-evdev.rules`.

**Security tradeoff:** `uaccess` on `uhid` lets any logged-in user create a
virtual HID device. That is acceptable on a single-user gaming image. On a
multi-user host, override the permissions rule with a group-based one in
`/etc/udev/rules.d/` using the same filename (a same-named file in `/etc` takes
priority over the one in `/usr/lib`):

```
KERNEL=="uinput", SUBSYSTEM=="misc", GROUP="input", MODE="0660"
KERNEL=="uhid",   SUBSYSTEM=="misc", GROUP="gearvr", MODE="0660"
```

Create the group, add only the user who runs the driver, and re-login:

```sh
sudo groupadd --system gearvr
sudo usermod -aG gearvr,input "$USER"
```

The `60-` and `70-` prefixes are deliberate: `71-seat.rules` consumes the
`uaccess` tag and `73-seat-late.rules` applies the ACL, so rules must sort before
them. A `99-` rule is too late.

## systemd user service

The packaged unit runs:

```sh
gearvr-cli run --backend evdev --motion
```

from `/usr/bin/gearvr-cli` (RPM) or `%h/.local/bin/gearvr-cli` (tarball). The
session supervisor owns reconnect, so a controller sleep or link drop does not
end the service; `Restart=on-failure` is a safety net for a crashed process.

Useful commands:

```sh
systemctl --user status gearvr.service
journalctl --user -u gearvr.service -f
systemctl --user restart gearvr.service
systemctl --user stop gearvr.service
```

`enable` starts the unit at graphical login through the user `default.target`.
On a headless host, enable lingering with `loginctl enable-linger "$USER"`.

### DualSense instead of evdev

The release binary already contains the `dualsense` backend. Use a drop-in so
the packaged unit stays untouched:

```sh
mkdir -p ~/.config/systemd/user/gearvr.service.d
cat > ~/.config/systemd/user/gearvr.service.d/dualsense.conf <<'EOF'
[Service]
ExecStart=
ExecStart=/usr/bin/gearvr-cli run --backend dualsense
EOF
systemctl --user daemon-reload
systemctl --user restart gearvr.service
```

For a tarball install, use `%h/.local/bin/gearvr-cli` in the drop-in instead. If
you have a magnetometer profile, add
`--mag-calibration %h/gearvr-captures/mag-calibration.toml --marg`. Without it
the service runs the same DualSense path in 6-DOF mode.

Verify with:

```sh
systemctl --user show -p ExecStart --value gearvr.service
ls /sys/bus/hid/devices/ | grep 054C
```

Do not run the DualSense backend and the evdev gamepad at the same time, or
Steam will see two controllers. Roll back by deleting the drop-in and running
`systemctl --user daemon-reload && systemctl --user restart gearvr.service`.

## Virtual DualSense notes

The virtual identity is VID/PID `054c:0ce6`, name `Wireless Controller`, MAC
`02:47:56:52:00:01`, `BUS_BLUETOOTH`. Steam sees a real DualSense and can use
its gyro. The virtual MAC is stable, so Steam keeps the same controller
configuration.

- `/dev/uhid` must be writable by the user; the packaged permissions rule and
  the tarball's `--udev` step handle that.
- `hid_playstation` creates three nodes: the gamepad, motion sensors, and
  touchpad. Reading the motion node as a non-root user can need an additional
  ACL; Steam reads the HID device directly and does not need it.
- Steam's default desktop layout does not bind the DualSense touchpad click. In
  **Settings → Controller → Desktop Layout → Edit Layout**, select the DualSense
  and bind **Touchpad Click → Left Mouse Click**.
- Rollback is omitting the flag: `--backend evdev` remains the default and
  creates only the `1209` devices.

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

## Release procedure

The release workflow lives in `.github/workflows/release.yml` and runs when a
GitHub release is **published** (not drafted). It also supports a manual
`workflow_dispatch` dry run that builds workflow artifacts without touching a
release.

1. Bump `[workspace.package] version` in `Cargo.toml`, then run
   `cargo check --workspace --all-features --locked` so `Cargo.lock` is current.
   Commit the change.
2. Tag the commit with `vMAJOR.MINOR.PATCH`, for example `v0.1.0`, and push the
   tag.
3. Create and publish a GitHub release for that tag. `gh release create v0.1.0
   --generate-notes` is one option; the web UI works too.
4. Wait for the Release workflow. The `verify` job checks the tag against
   `Cargo.toml`, then runs formatting, tests, and clippy. The `tarball` job builds
   in Ubuntu 22.04; the `rpm` job builds in Fedora 44; the `publish` job writes
   `SHA256SUMS` and attaches all three assets only after every build succeeds.
5. Download the assets and verify them:

   ```sh
   sha256sum -c SHA256SUMS
   ```

Artifact names:

```
gearvr-evdev-<version>-x86_64-linux-gnu.tar.gz
gearvr-evdev-<version>-1.x86_64.rpm
SHA256SUMS
```

If the tag does not match the workspace version, the `verify` job fails before
anything is uploaded. If an upload is interrupted, delete the release assets and
re-run the workflow for the same tag; the upload uses `--clobber`. To rehearse
the build without creating a release, use **Actions → Release → Run workflow**
and download the workflow artifacts.

## Troubleshooting

Start with the driver's own diagnostics:

```sh
gearvr-cli doctor
```

- **`error while loading shared libraries: libdbus-1.so.3`** — install the
  D-Bus client library (`dbus-libs` on Fedora, `libdbus-1-3` on Debian/Ubuntu).
- **`version 'GLIBC_2.xx' not found`** — the host is older than the documented
  glibc 2.35 baseline and is unsupported by the release artifacts.
- **`uhid writable: false` or missing `/dev/uinput` access** — install the udev
  rules and reconnect the controller. If the device nodes do not exist at all,
  load the kernel modules with `sudo modprobe uinput uhid`; the `uinput` rule
  also creates its node with `static_node=uinput`. Check
  `ls -l /dev/uinput /dev/uhid` and your session's ACL.
- **Service starts but cannot find the controller** — check
  `bluetoothctl info` reports `Paired: yes` and `Trusted: yes`; see Pairing.
- **Two controllers in Steam** — only one backend may run. Select evdev
  (default) or the DualSense drop-in, not both.
