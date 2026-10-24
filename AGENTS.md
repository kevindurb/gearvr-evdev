# AGENTS.md

Guidance for coding agents working in this repository. Start with `README.md`
and `docs/index.md`; the `docs/` tree is the research source of truth for the
controller and protocol.

## What this is

A clean-room Rust userspace driver for the Samsung Gear VR Controller
(ET-YO324). It consumes the controller's vendor GATT service over BlueZ/D-Bus
and presents it as Linux input devices (evdev/uinput, plus an opt-in virtual
DualSense over uhid). There is no kernel module. The target is Linux, especially
Steam Deck / SteamOS and Bazzite.

## Commands

Run from the repository root.

```sh
# Required before any change is considered done (same as CI)
cargo fmt --all --check
cargo test --workspace --all-features --locked
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings

# Default local build: vendored D-Bus client, no D-Bus headers needed
cargo build -p gearvr-cli
cargo build -p gearvr-cli --features dualsense   # include the uhid backend

# Release build: links the distribution's libdbus; needs pkg-config + dbus headers
cargo build --release --locked -p gearvr-cli \
    --no-default-features --features bluez,dualsense

# Shell scripts are checked by CI with sh -n / bash -n
sh -n packaging/install.sh
bash -n scripts/*.sh
```

Hardware-free ways to exercise the stack:

```sh
cargo run -p gearvr-cli -- synth --duration 20 --backend evdev --motion
cargo run -p gearvr-cli -- decode --hex '0800'
cargo run -p gearvr-cli -- replay capture.jsonl --include-tx --summary
```

`scan`, `doctor`, `capture`, `probe`, and `run` need a live BlueZ adapter and
usually a paired controller. Do not assume any of them can run in this
environment; prefer `synth`, `decode`, `replay`, and tests.

## Crate layout and boundaries

| Crate | Responsibility |
| --- | --- |
| `gearvr-protocol` | Pure packet parsing/building and constants. No I/O, no async, `no_std`-capable outside tests. |
| `gearvr-core` | `ControllerState`, link state, config/serde mirror types, the `Backend` trait. |
| `gearvr-fusion` | Axis mapping, gyro-bias estimation, 6-DOF Madgwick, optional MARG. |
| `gearvr-ble` | Pure command sequencer + fake transport; feature-gated `bluez-async` transport. |
| `gearvr-output` | Output backends: evdev/uinput, JSON/UDP, optional uhid DualSense. |
| `gearvr-cli` | `clap` CLI, session supervisor, wiring of the crates. |

Dependency direction follows that table. Keep `gearvr-protocol` free of I/O,
allocation (outside tests/the `testing` feature), and panics on untrusted input.
Keep hardware access behind the transport/backend traits so tests need neither a
controller nor a BlueZ adapter.

## Protocol invariants — do not regress

- Handshake is strict: write `0800`, wait for the `08 00` acknowledgement (allow
  up to ~3 s; 1 s is too short and 100–500 ms retries can corrupt it), then write
  `0100`. Never send both back to back.
- `0100` is not acknowledged. After the echo, enter `Streaming` immediately; a
  sensor packet before the echo is stronger evidence than the echo.
- Parse the suffix at `12 + 16*N`, never a fixed offset. The magnetometer is at
  suffix offset 32 (packet offset 48 with three samples).
- The vendor service UUID must decode to the ASCII bytes `Oculus Threemote`.
  A transposed byte pair makes BlueZ report the service as missing; there is a
  regression test for this.
- Preserve the fixes for variable sample counts, suffix offset, touchpad
  packing, and 13-bit vs 16-bit fields. Add a regression test for any protocol
  fix or new assumption, using synthetic packets.
- `gearvr-protocol` has `#![deny(unsafe_code)]`; do not introduce `unsafe` there.

## Testing expectations

- Tests must not require hardware, a controller, or a live BlueZ adapter.
  Use the fake transport, synthetic packet builders (`testing` feature), and
  fixtures under `crates/gearvr-protocol/tests/fixtures/`.
- Cover malformed/truncated packets, zero-sample packets, disconnect paths, and
  timeouts where relevant. Property tests live in
  `crates/gearvr-protocol/tests/properties.rs`.
- Capture files are JSON Lines (`v`, `t_us`, `dir`, `hex`) produced by
  `capture`/`probe`; `replay` decodes them offline.
- Do not claim hardware validation without the physical controller. Mark
  hardware-dependent tasks unchecked and record what was actually observed in
  `docs/implementation.md` instead.

## Style and error handling

- Rust 2021, MSRV 1.85 (`Cargo.toml`), stable toolchain with `rustfmt` and
  `clippy` (`rust-toolchain.toml`). Dependency versions are centralized in the
  workspace `[workspace.dependencies]`; add new dependencies sparingly.
- Libraries use `thiserror`; the CLI uses `anyhow`. Log through `tracing`, not
  `eprintln!`, except where stdout is deliberately machine-readable (JSON
  output) or part of a report subcommand.
- Rustfmt defaults. Keep comments focused on *why*, matching the existing
  style, and update doc comments when behavior changes.
- Preserve public API/serde shapes unless the change explicitly intends a break;
  captures and saved calibration profiles are versioned inputs.

## Specs, docs, and release

- Planned work goes through OpenSpec. Project context and rules are in
  `openspec/config.yaml`; active changes live in `openspec/changes/` and merge
  into `openspec/specs/` when archived. OpenCode commands are in `.opencode/`.
  `openspec validate --all` checks specs and changes.
- Update `docs/implementation.md` when validation status changes, and keep
  `README.md`/`packaging/README.md` in sync with CLI, build, and install changes.
- The release version lives in the workspace `Cargo.toml`; the tag must match it
  (`scripts/check-release-version.sh`). Do not edit `dist/` (ignored, generated).
- Never commit generated build output, captures with personal device data, or
  credentials.
