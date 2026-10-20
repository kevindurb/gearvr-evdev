# Tasks

## 1. Protocol fixtures, calibration, and hostile input

- [x] 1.1 Add a reusable notification fixture format and fixtures for zero-, one-, and three-sample sensor packets; verify the decoder tests pass with the suffix at the correct variable offset.
- [ ] 1.2 Implement the 66-byte `0300` calibration decoder as eight signed 21-bit triples plus a little-endian temperature, with padding-bit and range validation; verify valid, bad-padding, and out-of-range tests pass.
- [ ] 1.3 Decode both known settings record lengths and expose firmware bytes as ASCII and numeric values; verify the `QD4` firmware, short-record, and unknown-field tests pass.
- [ ] 1.4 Add property/fuzz-style coverage that feeds arbitrary and truncated byte slices to the parser; verify no test panics or hangs and unknown packets remain classified.
- [ ] 1.5 Document the packet lengths, suffix offset, calibration layout confidence, and fixture format in `docs/protocol.md` and `docs/implementation.md`; verify the docs match the tested behavior.
- [ ] 1.6 Run `cargo fmt --all --check`, `cargo test --workspace --all-features`, and `cargo clippy --workspace --all-targets --all-features -- -D warnings`; verify all pass.

## 2. Fusion calibration and axis configuration

- [ ] 2.1 Make the axis map configurable through the fusion config and CLI/config file; verify tests cover the documented map, an alternate map, and invalid map input.
- [ ] 2.2 Add a `pose-test` helper that captures raw accelerometer averages and gyro signs for the documented poses and prints a candidate map; verify synthetic poses produce the expected candidate and the helper never auto-applies it.
- [ ] 2.3 Apply validated factory calibration in the fusion path and report the active calibration source; verify tests cover valid calibration, rejected calibration, and online-bias fallback.
- [ ] 2.4 Harden fusion timing for timestamp wrap and long gaps; verify tests produce finite, normalized output for both cases.
- [ ] 2.5 Add a versioned magnetometer calibration file with hard/soft-iron data, board rotation, and validation metadata; verify load/save, corrupt-file, and version-mismatch tests pass.
- [ ] 2.6 Add optional MARG fusion that runs only with a validated magnetometer calibration; verify it is disabled by default and that calibrated synthetic field data updates yaw without destabilizing the 6-DOF fallback.
- [ ] 2.7 Document the frame convention, calibration sources, persistence format, and fallback behavior in `docs/calibration.md`; verify the commands and file example match the implementation.
- [ ] 2.8 Run `cargo fmt --all --check`, `cargo test --workspace --all-features`, and `cargo clippy --workspace --all-targets --all-features -- -D warnings`; verify all pass.

## 3. Gesture detection and evdev input behavior

- [ ] 3.1 Add a pure, time-injected touchpad gesture detector with configurable tap and swipe thresholds; verify synthetic tests cover tap, swipe, slow drag, diagonal jitter, and lift without movement.
- [ ] 3.2 Map detected gestures to documented discrete gamepad events without suppressing raw touchpad position or click events; verify event-translation tests and the default-disabled behavior.
- [ ] 3.3 Reset held keys, touchpad state, and motion axes when the session ends; verify tests cover disconnect while a key is held and verify the device remains present.
- [ ] 3.4 Document the evdev button/axis map, gesture codes, thresholds, and volume opt-in in `docs/linux-integration.md` and `README.md`; verify the documented map matches the backend.
- [ ] 3.5 Run `cargo fmt --all --check`, `cargo test --workspace --all-features`, and `cargo clippy --workspace --all-targets --all-features -- -D warnings`; verify all pass.

## 4. Session supervisor and live diagnostics

- [ ] 4.1 Add a live-session supervisor that owns transport lifecycle, bounded reconnect backoff, and backend lifetime while reusing a fake transport in tests; verify repeated disconnect/reconnect cycles preserve one backend session and do not accumulate retry state.
- [ ] 4.2 Drive acknowledgement timeout, retry budget, stream-idle recovery, and slow-mode fallback through a fake clock; verify tests prove `0100` is never sent before acknowledgement and that retry limits are enforced.
- [ ] 4.3 Add configurable keep-alive scheduling and a disable option; verify fake-clock tests send `0400` only at the configured interval and never when disabled.
- [ ] 4.4 Extend the diagnostic command to report adapter availability, pairing/trust state, `/dev/uinput` and `/dev/uhid` access, and recovery actions; verify output tests for missing permissions, missing adapter, and missing pairing agent.
- [ ] 4.5 Document pairing, trusting, reconnect, sleep/wake behavior, and the remove-and-repair recovery path in `packaging/README.md` and `docs/linux-integration.md`; verify the commands are copy-pasteable and match the diagnostics.
- [ ] 4.6 Run `cargo fmt --all --check`, `cargo test --workspace --all-features`, and `cargo clippy --workspace --all-targets --all-features -- -D warnings`; verify all pass.

## 5. Output contracts and periodic backend ticks

- [ ] 5.1 Add a default no-op periodic tick and interval hint to the backend interface and update every backend; verify the workspace compiles and existing backend tests still pass.
- [ ] 5.2 Verify the JSON and UDP contracts with tests for exactly one object per sensor packet, orientation absent before fusion, machine-readable stdout, and datagram delivery to a local receiver.
- [ ] 5.3 Verify stable gamepad and motion-node identity across reconnect and the reset behavior on disconnect; add an integration-style fake-backend test if the current tests do not cover it.
- [ ] 5.4 Document the JSON schema, UDP behavior, motion-node axes/ranges, and default identity in `README.md` and `docs/linux-integration.md`; verify each documented field is emitted by a test.
- [ ] 5.5 Run `cargo fmt --all --check`, `cargo test --workspace --all-features`, and `cargo clippy --workspace --all-targets --all-features -- -D warnings`; verify all pass.

## 6. Virtual DualSense backend

- [ ] 6.1 Add a feature-gated `dualsense` output dependency on `uhid` and implement a pure report builder with Bluetooth CRC32; verify known CRC vectors and report-shape tests pass.
- [ ] 6.2 Add a fake uhid transport and backend tests for the 16 ms continuous report cadence, neutral report on disconnect, and unchanged-state reporting; verify no report is emitted when the backend is disabled.
- [ ] 6.3 Wire an opt-in CLI/config selection for the DualSense backend with trigger, Home, Back, touchpad, battery, and fused motion mapping; verify mapping tests and that the evdev backend is unaffected.
- [ ] 6.4 Document the DualSense identity, Steam setup, expected gyro behavior, and rollback to evdev in `docs/linux-integration.md` and `packaging/README.md`; verify the documented flags exist.
- [ ] 6.5 Run `cargo fmt --all --check`, `cargo test --workspace --all-features`, and `cargo clippy --workspace --all-targets --all-features -- -D warnings`; verify all pass.

## 7. Packaging, permissions, and service

- [ ] 7.1 Add or confirm a udev rule for `/dev/uhid` access without weakening unrelated permissions; verify the rule syntax with `udevadm verify` and confirm it is ordered after seat/input rules where required.
- [ ] 7.2 Update the systemd user unit to run the selected backend from the documented install path, wait for Bluetooth, and restart on failure; verify `systemd-analyze verify` accepts the unit.
- [ ] 7.3 Update `packaging/README.md` with install, pair, trust, enable, stop, journal, and remove-and-repair steps; verify every command exists and the paths match the unit and udev files.
- [ ] 7.4 Update `docs/implementation.md` with the new feature status and clearly label all hardware-unvalidated claims; verify the status table matches the tasks left unchecked.
- [ ] 7.5 Run `cargo fmt --all --check`, `cargo test --workspace --all-features`, and `cargo clippy --workspace --all-targets --all-features -- -D warnings`; verify all pass.

## 8. Hardware validation (blocked until a physical ET-YO324 is available)

> Leave these unchecked until each item has actually been exercised. Record the date, host, kernel, BlueZ version, and observed result.
>
> First session: 2026-10-20, `steamdeck`, kernel 7.2.8, BlueZ 5.87, unit
> `2C:BA:BA:4F:00:14` firmware `QD4`. Results are recorded in
> `docs/implementation.md`.

- [x] 8.1 Pair, trust, and connect the controller; verify `scan` finds it and `run --backend evdev` completes the `0800` → `08 00` → `0100` handshake.
- [x] 8.2 Capture real sensor, settings, and `0300` calibration notifications; add them as named fixtures and verify replay through `decode` matches the captured values.
- [x] 8.3 Run the pose test and compare the candidate axis map with the default; verify nose-up, nose-down, left-side, and right-side signs match the documented frame.
- [ ] 8.4 Exercise at least ten sleep/wake or disconnect/reconnect cycles; verify the same virtual devices survive, inputs release, and a fresh handshake completes each time.
- [x] 8.5 Verify Steam sees the evdev gamepad with the expected mapping and no stuck inputs; verify the gesture defaults are usable or tune the thresholds.
- [ ] 8.6 Verify Steam Input enumerates the virtual DualSense and exposes gyro; confirm continuous reports prevent timeout and that disabling the backend restores plain evdev behavior.
- [ ] 8.7 Install and run the systemd user service on the Deck; verify start at login, restart after a failed session, journal output, and clean stop.
- [ ] 8.8 Record the hardware results and any deviations in `docs/implementation.md`; verify every claim marked validated has an observed result.

## 9. Integration verification

- [ ] 9.1 Run the full workspace checks with all features: `cargo fmt --all --check`, `cargo test --workspace --all-features`, and `cargo clippy --workspace --all-targets --all-features -- -D warnings`; verify all pass.
- [ ] 9.2 Smoke-test `synth` against the evdev backend and `decode` against the fixture corpus; verify the virtual device is readable with `evtest`/`jstest` where available.
- [ ] 9.3 Review every requirement in `specs/` against the implemented tests and docs; verify no requirement is left without a task or a documented hardware blocker.
