# Tasks

## 1. Build portability and D-Bus linkage

- [x] 1.1 Split the D-Bus choice from the BlueZ feature: keep the vendored build available and default for local source builds, and add a release feature combination that links the system D-Bus client library. Verify `cargo build -p gearvr-cli` still succeeds without D-Bus development headers, and `cargo build -p gearvr-cli --no-default-features --features bluez,dualsense` succeeds with `libdbus-1-dev`/`pkg-config` installed.
- [x] 1.2 Add `scripts/build-tarball.sh` to build the release feature set with `--locked`, strip the binary, and stage the archive layout from design.md. Verify it runs inside an Ubuntu 22.04 container and that `tar -tzf` lists the expected files.
- [x] 1.3 Verify the staged binary's portability in the build script: `readelf -d` lists `libdbus-1.so.3`, and the highest imported `GLIBC_*` version is at most `GLIBC_2.35`. The script exits non-zero if either check fails.
- [x] 1.4 Document the two D-Bus build modes and their prerequisites in `README.md` and `packaging/README.md`; verify every documented command runs as written.
- [x] 1.5 Run `cargo fmt --all --check`, `cargo test --workspace --all-features`, and `cargo clippy --workspace --all-targets --all-features -- -D warnings`; verify all pass.

## 2. Tarball and user installation

- [x] 2.1 Add `packaging/install.sh` with `--user`, `--udev`, and `--uninstall` modes, including the non-root guard, the `systemctl --user` check, and the printed privileged udev command. Verify `bash -n` passes and a `--user` install into a temporary `HOME` with a stubbed `systemctl` places the binary and unit correctly and is repeatable.
- [x] 2.2 Add `packaging/udev/60-gearvr-permissions.rules` for `/dev/uinput` and `/dev/uhid`, and extend the output rule for the `1209:0003` companion keyboard if validation shows it lacks access. Verify `udevadm verify` accepts both files and the filenames sort before `71-seat.rules`.
- [x] 2.3 Add the RPM-path user unit transformation (or a packaged unit variant) so the packaged `ExecStart` is `/usr/bin/gearvr-cli`; verify `systemd-analyze verify` accepts the transformed unit.
- [x] 2.4 Update `packaging/README.md` with tarball install, udev installation, uninstall, migration from a manual install, and the `uaccess`/`uhid` security tradeoff. Verify each command and path matches the script and unit.

## 3. RPM package

- [x] 3.1 Add `packaging/rpm/gearvr-evdev.spec` using the release tarball as `Source0`, a prebuilt-binary `%install`, no distro suffix, runtime requirements on `dbus-libs`, `systemd`, and `bluez`, and udev reload scriptlets guarded for image builds. Verify `rpmbuild -bb` succeeds in a Fedora 44 container.
- [x] 3.2 Add `scripts/build-rpm.sh` to read the version from `Cargo.toml`, pass it to `rpmbuild`, and write `gearvr-evdev-<version>-1.x86_64.rpm` to `dist/`. Verify the filename and `rpm -qpR`/`rpm -qpl` output match the design.
- [x] 3.3 Install the RPM in a clean Fedora container with no running systemd or udev, verify installation succeeds, the service is not enabled, and all packaged files are present; then remove it and verify user captures and calibration files are untouched.
- [x] 3.4 Update `packaging/README.md` with RPM install, enable, removal, a bootc Containerfile snippet that installs the RPM and runs `systemctl --global enable gearvr.service`, and the single-user/multi-user caveat. Verify the documented paths and commands against the built RPM.

## 4. GitHub Actions and release publishing

- [x] 4.1 Add `.github/workflows/ci.yml` for pushes to `main` and pull requests: formatting, workspace tests, clippy, and a portable-build job that runs the release feature set and portability checks. Verify with `actionlint` (or an equivalent workflow check) and a branch run.
- [x] 4.2 Add `scripts/check-release-version.sh` that compares a release tag with the workspace version and fails with both values on a mismatch. Verify it accepts `v0.1.0` and rejects `v0.2.0` against the current tree.
- [x] 4.3 Add `.github/workflows/release.yml` triggered by `release: types: [published]`, with read-only default permissions, `contents: write` only in the publish job, a version-check job, the tarball and RPM jobs, `SHA256SUMS` generation, and `gh release upload` only after every build succeeds. Add a `workflow_dispatch` dry-run path that uploads workflow artifacts without publishing. Verify with `actionlint` and a dry run.
- [x] 4.4 Document the release procedure in `packaging/README.md`: bump the workspace version, tag `vMAJOR.MINOR.PATCH`, publish the release, confirm both assets and the checksum file, and recover from a failed upload. Verify the steps match the workflow triggers and artifact names.
- [x] 4.5 Exercise the release path with a test release once the version is final: verify both assets and `SHA256SUMS` are attached, and verify a deliberately mismatched tag fails before uploading. Record the observed result.

## 5. Integration verification and hardware validation

- [x] 5.1 Extract the tarball in a Debian 12 container and run `gearvr-cli info` and `gearvr-cli doctor`; verify the supported-library baseline holds and the diagnostics report missing device permissions clearly.
- [x] 5.2 Run `systemd-analyze verify` on both unit variants and `udevadm verify` on both rule files in the packaging containers; verify no errors.
- [x] 5.3 Run the full workspace checks and the release feature build one final time; verify `cargo fmt --all --check`, `cargo test --workspace --all-features`, and `cargo clippy --workspace --all-targets --all-features -- -D warnings` all pass.
- [x] 5.4 Update `docs/implementation.md` with the release-artifact status and clearly leave hardware claims unvalidated; verify the status table matches the tasks left unchecked.
- [x] 5.5 Review the `release-packaging` spec requirements against the tests, workflows, and documentation; verify no requirement is left without coverage or a hardware blocker.
- [ ] 5.6 Hardware validation only: install the RPM or tarball on a Deck or bootc host, pair and trust the controller, enable the user service, and verify a live handshake and stream with both evdev and DualSense selection. Record the date, host, kernel, BlueZ version, and observed result in `docs/implementation.md`. Leave this unchecked until performed.
