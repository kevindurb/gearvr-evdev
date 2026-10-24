# Design

## Context

See `proposal.md` for motivation. The current release process is manual: build
on the development host, copy the binary to the target, install the udev rule as
root, and copy the user unit into place. The build host currently produces a
binary that imports `GLIBC_2.43` symbols, and the `bluez` feature unconditionally
enables the vendored D-Bus C library, so the resulting binary bundles a private
copy of `libdbus` 1.14.4.

The existing user unit hardcodes `%h/.local/bin/gearvr-cli`. The existing udev
rule covers the `1209:0001` gamepad and `1209:0002` motion nodes but not the
`/dev/uinput` and `/dev/uhid` device nodes the driver opens. On the development
host those are granted by unrelated ublue/Sunshine rules; a stock Fedora or
generic bootc base does not guarantee them.

There are no tags, no release artifacts, and no `.github/` directory. The
workspace version is `0.1.0` and `Cargo.lock` is committed.

No new protocol behavior is introduced. `gearvr-protocol` remains I/O-free and
untouched.

## Goals / Non-Goals

**Goals:**

- One reproducible build produces a portable x86_64 binary with a documented
  glibc floor and no bundled D-Bus client library.
- The same binary is wrapped as a tarball for arbitrary distributions and as a
  prebuilt-binary RPM for Fedora and bootc images.
- A published GitHub release automatically receives both artifacts plus a
  checksum file, and cannot receive artifacts built from a mismatched tag.
- Installation and bootc enablement are documented and testable without the
  controller.

**Non-Goals:**

- A source RPM, Fedora dist-git package, signing, or a COPR repository.
- Static musl builds, multi-architecture artifacts, or a runtime container
  image.
- Renaming the binary, changing driver behavior, or introducing a Rust
  `install` subcommand.
- Auto-enabling the user service from the RPM.

## Decisions

### 1. Build baseline: Ubuntu 22.04, glibc 2.35, system libdbus

Build the release binary in an `ubuntu:22.04` container with `pkg-config` and
`libdbus-1-dev`. This yields a binary whose maximum imported glibc version is
2.35 and whose dynamic dependency is the system `libdbus-1.so.3`.

Why 2.35: it covers Debian 12 (2.36), Ubuntu 22.04 (2.35), Fedora 38 and newer,
and current SteamOS/Bazzite bases, while avoiding the 2.43 floor of the current
development host. RHEL 9 (glibc 2.34) is outside the supported set.

Alternatives considered:

- Build on Fedora 44: reproduces the current portability problem.
- Static musl: attractive for a truly single file, but the vendored libdbus C
  build under musl is unproven and would bundle a C library. Deferred.
- Per-distribution builds: more artifacts and maintenance than the project
  needs for 0.1.0.

The release job builds with Rust 1.85, the declared minimum, to catch accidental
MSRV drift. Checks jobs use the repository's stable toolchain.

### 2. D-Bus linkage: separate the vendored build from the BlueZ feature

Split the current feature wiring:

- `bluez` enables the async BlueZ transport and no longer forces
  `libdbus-sys/vendored`.
- A separate `vendored-dbus` feature enables the private libdbus build and is
  part of the CLI's default features, preserving today's zero-setup local
  builds.
- Release builds use `--no-default-features --features bluez,dualsense`, which
  selects the system `libdbus-1` through `pkg-config`.

This keeps local development working on machines without D-Bus development
headers, while ensuring published artifacts do not bundle a C library whose
license is AFL-2.1 or GPL-2.0+ and whose security updates would otherwise be
frozen into the binary.

The release build additionally enables `dualsense`, so a packaged binary can
select the virtual DualSense backend at runtime without a rebuild.

### 3. Tarball layout and installer

```
gearvr-evdev-0.1.0/
  gearvr-cli
  install.sh
  README.md
  INSTALL.md
  LICENSE
  systemd/gearvr.service
  udev/70-gearvr-evdev.rules
  udev/60-gearvr-permissions.rules
```

`install.sh` is a small POSIX shell installer with three explicit modes:

```
./install.sh --user        # binary -> ~/.local/bin, unit -> ~/.config/systemd/user
                           # daemon-reload, enable --now; then prints the udev step
sudo ./install.sh --udev   # rules -> /etc/udev/rules.d, reload and trigger
./install.sh --uninstall   # disable and remove the user binary and unit
```

`--user` refuses to run as root so files do not end up owned by root in a
user's home. `--udev` requires root. The user mode checks `systemctl --user`
availability and reports the exact command to enable after login if no user
manager is reachable. `--uninstall` preserves `~/.local/share/gearvr-captures`
and any calibration profiles.

The existing `packaging/systemd/gearvr.service` is used for the user install.
The RPM build transforms its `ExecStart` to `/usr/bin/gearvr-cli` rather than
duplicating the unit; the transformed file is checked with
`systemd-analyze verify`.

Alternative considered: a Rust `install` subcommand. It would be more testable
but adds CLI surface and asset embedding for little benefit at 0.1.0. Deferred.

### 4. RPM: a prebuilt-binary package with no enablement

Add `packaging/rpm/gearvr-evdev.spec`:

- `Name: gearvr-evdev`, `Version` supplied by the release script from
  `Cargo.toml`, `Release: 1`, no `%{?dist}` suffix, `BuildArch: x86_64`.
- `Source0` is the release tarball; `%prep` extracts it, `%build` is empty, and
  `%install` copies the prebuilt binary and assets. No Rust toolchain or
  D-Bus headers are required in the RPM build.
- Runtime requirements: `dbus-libs`, `systemd`, and `bluez`.
- Files: `%{_bindir}/gearvr-cli`, `%{_userunitdir}/gearvr.service`,
  `%{_udevrulesdir}/70-gearvr-evdev.rules`,
  `%{_udevrulesdir}/60-gearvr-permissions.rules`, `%license LICENSE`, and the
  install documentation.
- Scriptlets only reload udev when `/run/udev` exists; they never call
  `systemctl` and never enable the service. That makes installation safe in a
  bootc image build, where systemd and udev are not running.
- Removal leaves user configuration, captures, and calibration profiles alone.

The artifact filename is deliberately distro-neutral:
`gearvr-evdev-0.1.0-1.x86_64.rpm`. The binary has no compiled-in Fedora
dependency, so this package is valid on any RPM-based host with the declared
runtime libraries.

Alternatives considered: a source RPM (rejected for 0.1.0 because it requires
offline crate vendoring and a full build in the RPM), and a COPR-only path
(deferred; the release asset is enough for bootc `dnf install`).

### 5. udev rules: split output access from creation access

Keep `70-gearvr-evdev.rules` for the driver's own input nodes and add
`60-gearvr-permissions.rules`:

```
KERNEL=="uinput", SUBSYSTEM=="misc", TAG+="uaccess", OPTIONS+="static_node=uinput"
KERNEL=="uhid",   SUBSYSTEM=="misc", TAG+="uaccess"
```

The `70-` output rule stays before `71-seat.rules`; the `60-` permission rule is
also before the seat rules. Both are installed into
`/usr/lib/udev/rules.d` by the RPM and `/etc/udev/rules.d` by the tarball's
`--udev` mode.

`uaccess` on `uhid` lets any logged-in user create a virtual HID device. That is
acceptable for a single-user gaming image and is documented as a deliberate
tradeoff, with the group-based alternative for multi-user hosts. The companion
keyboard node (`1209:0003`) is checked and given an explicit rule if it does not
receive `uaccess` automatically as an input keyboard.

### 6. GitHub Actions: checks plus release-triggered publishing

```
push / pull_request                         release published
        |                                         |
        v                                         v
  +----------------+                    +----------------------+
  | checks         |                    | verify tag == version|
  | fmt/test/clippy|                    | fmt/test/clippy      |
  +----------------+                    +----------+-----------+
                                                   |
                              +--------------------+--------------------+
                              v                                         v
                    +------------------+                      +------------------+
                    | tarball job      |                      | rpm job          |
                    | ubuntu:22.04     |                      | fedora:44        |
                    | build + verify   |                      | rpmbuild only    |
                    +--------+---------+                      +--------+---------+
                             |                                         |
                             +------------------+----------------------+
                                                v
                                      +-------------------+
                                      | publish job       |
                                      | checksums + gh    |
                                      | release upload    |
                                      +-------------------+
```

Workflows:

- `.github/workflows/ci.yml` on pushes to `main` and pull requests:
  formatting, workspace tests, and clippy; plus a portable-build job in
  `ubuntu:22.04` that builds the release feature set and checks the glibc floor
  and the `libdbus-1.so.3` dependency. The portable build catches the failure
  mode that motivated this change before a release.
- `.github/workflows/release.yml` on `release: types: [published]` with
  `contents: write`; a `workflow_dispatch` path builds and uploads workflow
  artifacts without publishing, for dry runs.

The release workflow:

1. Compares `${GITHUB_REF_NAME#v}` with `cargo metadata --locked --no-deps`.
   Any mismatch fails before a build starts.
2. Builds the tarball in `ubuntu:22.04` with `--locked --no-default-features
   --features bluez,dualsense`, strips the binary, and stages the installer and
   assets.
3. Verifies the binary: `readelf -d` includes `libdbus-1.so.3`, and the highest
   `GLIBC_*` version is at most `GLIBC_2.35`.
4. Uploads the tarball as a workflow artifact; the RPM job downloads it and
   runs `rpmbuild -bb` in `fedora:44` with `rpm-build` and
   `systemd-rpm-macros`, then verifies the file list and requirements.
5. The publish job downloads both artifacts, writes `SHA256SUMS`, and uploads
   all three with `gh release upload "$TAG" ... --clobber` using the workflow
   token.

Publish runs only after every build and verification job succeeds. Artifacts
are uploaded only after all are built, so a failed build cannot leave a partial
release. Actions are pinned to major versions for 0.1.0; pinning to commit SHAs
is a follow-up hardening step. Signing is out of scope, so the checksum file is
the only integrity mechanism.

### 7. bootc enablement

The RPM intentionally leaves the user service disabled. A bootc build installs
the release RPM and enables it explicitly:

```dockerfile
RUN dnf install -y /tmp/gearvr-evdev-0.1.0-1.x86_64.rpm \
 && dnf clean all
RUN systemctl --global enable gearvr.service
```

`systemctl --global enable` works offline and creates
`/etc/systemd/user/default.target.wants/gearvr.service`. This starts the service
for every interactive user, so the documentation states the single-user
assumption and tells multi-user image builders to enable it per user instead.
The service still starts only when a user manager starts; it does not run at
boot without a login.

### 8. Migration from the manual install

The manual install puts the binary in `~/.local/bin` and the unit in
`~/.config/systemd/user`; the RPM puts them in `/usr`. A host can end up with
both, and the user unit takes precedence. The documentation tells users to
uninstall one before installing the other, and shows how to verify which
binary and unit are active. No data migration is needed: captures and
calibration profiles are untouched by either path.

## Risks / Trade-offs

- **Ubuntu 22.04 image drift changes the glibc floor** → Pin the container image
  by digest or at least by release tag, and check the imported glibc versions in
  CI on every release.
- **libdbus 1.12 headers on Ubuntu vs 1.16 at runtime** → Only the stable
  `libdbus-1.so.3` ABI is used; the runtime library is newer and backward
  compatible. The RPM and tarball both declare the dependency.
- **Rust 1.85 may not compile a transitive dependency resolved by a newer
  Cargo** → The committed lockfile is used with `--locked`; CI builds with 1.85.
  If a dependency raises its MSRV, either pin it or deliberately raise the
  workspace MSRV before release.
- **Prebuilt-binary RPM is not Fedora-review compliant** → It is documented as a
  bootc/COPR-oriented artifact, not a Fedora package. A source RPM can be added
  later without changing the user-facing layout.
- **Actions supply chain** → Use the workflow token with `contents: write` only
  in the publish job, keep default permissions read-only elsewhere, and pin
  actions to SHAs as a follow-up.
- **Partial release uploads** → Build and verify every artifact before the
  publish job starts; if the upload still fails, delete the release assets and
  re-run the workflow for the same tag.
- **`uaccess` on `/dev/uhid` is a security tradeoff** → Document it, keep the
  rule separate so it can be removed, and describe the group-based alternative.
- **Global enablement on a multi-user bootc image causes connection contention**
  → Document the single-user assumption and the per-user enablement path.
- **Manual and packaged installs can coexist** → Document the migration and make
  the installed unit's path visible in diagnostics.

## Migration Plan

1. Land the feature split and rebuild locally to confirm the default source
   build still works without `dbus-devel`.
2. Add the packaging assets and build a tarball and RPM locally in the pinned
   containers; verify the file lists and the glibc/`libdbus` checks.
3. Add the workflows, run the checks workflow on a branch, then create a test
   release to confirm assets are attached.
4. Update `packaging/README.md` and `docs/implementation.md`, then install the
   RPM on a Deck or bootc host as the separate hardware validation step.

Rollback for users is removing the RPM or running the tarball's uninstall mode.
Rollback for a bad release is deleting its assets and re-running the workflow
from a corrected tag; bootc users roll back the OS image.
