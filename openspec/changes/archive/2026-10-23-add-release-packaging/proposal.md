# Proposal

## Why

The driver can be built and run from a source checkout, but there is no
artifact a user or a bootc image can consume without a Rust toolchain. The
current release binary also inherits the build host's glibc requirement, so it
runs only on machines at least as new as the machine that produced it. A
repeatable release path is needed before the driver can be recommended outside
the developer's own Deck.

## What Changes

- Add a portable release build that targets an explicit old-glibc baseline
  (x86_64, glibc 2.35 via an Ubuntu 22.04 build container) and links the
  system `libdbus-1` instead of bundling a private copy.
- Separate the D-Bus client-library choice from the BlueZ feature: release
  packages link the distribution's D-Bus client library, while the vendored
  build remains available and stays the default for local source builds.
- Produce two release artifacts from one build:
  - a `.tar.gz` containing the binary, the user service unit, udev rules, an
    install script, documentation, and license; and
  - a prebuilt-binary RPM that installs the binary, user unit, and udev rules
    into `/usr` for Fedora and bootc images.
- Add a permissions udev rule covering `/dev/uinput` and `/dev/uhid` so a fresh
  install works without a manual rule, and document the security tradeoff of
  `uaccess` on `uhid`.
- Add GitHub Actions: a checks workflow for pushes and pull requests, and a
  release workflow triggered by a published GitHub release that builds the
  tarball and RPM, writes checksums, and attaches the assets to that release.
- Verify release tags against the workspace version so an artifact cannot be
  published under the wrong tag.
- Document tarball, RPM, and bootc installation, enablement, and removal,
  including the single-user assumption behind a globally enabled user service.

## Capabilities

### New Capabilities

- `release-packaging`: release artifact contents, build portability baseline,
  tarball and RPM installation behavior, udev/service assets, tag/version
  checks, and GitHub release attachment behavior.

### Modified Capabilities

None.

## Impact

- `Cargo.toml` / `crates/gearvr-ble/Cargo.toml`: D-Bus feature split; release
  profile and artifact metadata if needed.
- `packaging/`: tarball installer, RPM spec, user-unit variants, permissions
  udev rule, and installation documentation.
- `.github/workflows/`: new checks and release workflows.
- `README.md` and `docs/implementation.md`: install path and validation status.
- No changes to protocol parsing, fusion, or backend behavior.

## Assumptions

- The supported release platform is x86_64 Linux with glibc 2.35 or newer and
  the system `libdbus-1.so.3`; BlueZ already requires that library.
- The RPM packages the prebuilt binary rather than rebuilding from source. It
  is intended for bootc/COPR use, not Fedora package review.
- The installed binary keeps its current name, `gearvr-cli`.
- The tarball ships a small install script; a Rust `install` subcommand is
  deferred.
- The user service is not enabled automatically by the RPM. bootc images
  enable it explicitly and are expected to have one interactive user.

## Non-goals

- Static musl builds, multi-architecture artifacts, AppImage, Flatpak, or a
  system-wide service.
- Source RPMs, Fedora dist-git submission, package signing, or a COPR repo.
- Renaming the binary or changing the service's runtime behavior.
- Hardware validation beyond a final install-and-stream smoke test; that task
  stays separate and unchecked until performed on a Deck or bootc host.

This change is hardware-free; only the final validation task needs the
controller.
