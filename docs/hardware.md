# Hardware

Model: **Samsung Gear VR Controller ET-YO324** (US SKU `ET-YO324BBEGUS`).
Released April 2017 alongside the SM-R324 Gear VR with Controller.
Discontinued; Samsung no longer stocks it.

## Physical

| Property | Value |
| --- | --- |
| Dimensions | 4.25 × 1.88 × 1.49 in (108 × 48 × 38 mm) |
| Weight | 37 g (1.3 oz) with batteries |
| Colour | Black |
| Power | 2 × AAA (not rechargeable, no charge port) |
| Indicator | Tri-colour LED (pairing / connected / low battery) |
| Mount | Wrist strap loop + a holder that clips to the Gear VR headset |

Sources: Samsung retail datasheet (ET-YO324BBEGUS), Samsung user manual Rev 1.1 (04/2017).

### Inputs

- Clickable capacitive **touchpad**, circular, ~36 mm diameter.
  Reported coordinate space is square, 0–320 full scale (see `packets.md`).
- **Trigger** — index-finger, at the far end (the end you point).
- **Home** — concave, easy to find blind.
- **Back** — raised.
- **Volume up / down** rocker.
- **No haptics, no microphone, no speaker.** Verified by absence of any audio
  service in the GATT table, BLE-only advertising, and by checking that
  accelerometer noise is unchanged across every command (a vibration motor
  spinning up would show in the sample-to-sample noise floor).

### Power / power states

- The controller has **no on/off switch**.
- It enters sleep after inactivity and wakes on any button, trigger or touchpad press.
- **It will disconnect the BLE link on inactivity regardless of keep-alive
  commands.** Regular button presses are the only reliable way to stay connected.
- Indicator flashes red when the AAA cells are low.

## Radio and regulatory

| Property | Value |
| --- | --- |
| Standard | Bluetooth 4.2 Low Energy (BLE only, no Classic) |
| Band | 2402–2480 MHz |
| Conducted output | ~1 mW (0 dBm) |
| FCC ID | `A3LETYO324` (grantee code `A3L`, product code `ETYO324`) |
| FCC filing date | 2017-02-19, grant 2017-02-19, exhibits public 2017-08-18 |
| Test lab | SGS Korea |

The FCC application included block diagram, schematics and operational
description, but all three are **metadata-only in the public record**
(short-term + long-term confidentiality granted), so they are not readable.

## Silicon

| Role | Part | Package | Notes |
| --- | --- | --- | --- |
| MCU + radio | **Dialog Semiconductor DA14681** (now Renesas SmartBond) | WLCSP53, 3.41 × 3.01 mm | Cortex-M0, BLE 4.2, 64 kB OTP, 128 kB data SRAM, 16 kB cache SRAM, 128 kB ROM, integrated PMU/battery charger/fuel gauge, hardware crypto engine. "Not recommended for new designs"; superseded by DA14682/83. |
| Flash | **GD25LQ32** (GigaDevice, 32 Mbit / 4 MB SPI NOR) | USON8 4×4 mm | Holds firmware. Early teardowns misidentified this as a MOSFET — it is the SPI flash. |
| IMU (accel + gyro) | ST-family 6-axis, exact part unconfirmed. Candidates: LSM6DS3 / LSM6DSL / LSM9DS-family. One writeup guesses "LSM9DS3". | LGA-14L ~2.5 × 3 mm | Confirmed only by **sensitivity**: 2048 LSB/g at ±16 g and 14.285 LSB/(°/s) at ±2000 dps, which are ST datasheet values. |
| Magnetometer | **AK09912** (Asahi Kasei) | WL-CSP / BGA 1.6 × 1.6 mm | Mounted on the **touchpad board**, not with the IMU. |
| Touchpad controller | unknown | — | Not yet identified. |

### The split-sensor problem

The IMU and the magnetometer are on **different boards**:

- accelerometer + gyroscope live in the handle,
- the magnetometer sits up on the touchpad assembly,
- the two boards are at roughly **30°** to each other.

If you fuse them naively, yaw swings as you tilt the controller. The fix found
empirically is to rotate the magnetometer vector by −30° about the relevant axis
and negate its X component before fusion. This is a **per-unit-ish calibration**,
not a specification — see `open-questions.md`.

## Firmware

### Versions observed in the wild

| Version string | Notes |
| --- | --- |
| `YO324XXU0AQC1` | Common; shipped on launch units |
| `YO324XXU0AQD4` | Later; also common |
| `YO324XXU0AQBG` | Rare; seen on one unit only, no further references |

The firmware revision is readable from GATT Device Information `0x2A26` (a static
"0000" on some units) and, properly, from the `0500` device-record command
(see `packets.md`), which also exposes the real serial number.

### SUOTA (over-the-air firmware update)

- Firmware images ship **inside Samsung's Android app**
  `com.samsung.android.app.vr.input.service` as assets:
  `assets/firmware/downgrade/FOTA_YO324XXU0AQC1.img` and
  `assets/firmware/upgrade/FOTA_YO324XXU0AQD4.img`.
- These are **Dialog SUOTA** payloads, not raw binaries, and they are **not
  signature-checked** — unlike the Daydream controller, which uses SHA-256-signed
  updates. This means **downgrade works**.
- They flash successfully with Dialog's stock smartphone SUOTA app, using:
  - MISO `P0_2`, MOSI `P0_1`, CS `P0_5`, SCK `P0_0`
  - Image bank 0 (default)
  - Block size 240 (default)
- The controller does not appear in the Dialog app's list until you enable
  "Bonded" or "All nearby devices" in the app's overflow menu.
- The `FEF5` SUOTA GATT service is always advertised even when the device is not
  visibly listable.

**This is the one genuine door to changing hardware behaviour** — and the fastest
way to brick a controller. Not in scope for this project, but worth knowing.

### Debug / recovery pads

Five test pads under a sticker beneath the battery, reachable **without
disassembly**:

| Pin | Signal (per DA14681 datasheet) |
| --- | --- |
| 1 | 3.3 V |
| 2 | ADC2 / P1_3 (`UTX`) |
| 3 | P2_3 (`URX`) |
| 4 | GND |
| 5 | RST |

- ~1 kΩ series resistors between pads and SoC pins, plus pulldowns on the IO pins.
- These correspond to two DA14681 boot modes:
  - **Boot 5 — UART:** UTX = P1_3, URX = P2_3. Reading at startup yields a
    repeating `F8 A2 FA C4 D3 1A` preamble, *not* the module ID the datasheet says
    to expect. No commands were successfully accepted over it.
  - **Boot 6 — SPI slave:** MISO P0_2, MOSI P0_1, SCK P0_0, CS P0_5. Same pins as
    the SUOTA config. **Untested** as a reflash path. Note the DA14681 has OTP
    memory and it is unknown whether the OTP is already burned, which would limit
    what can be reflashed.
- Firmware strings mention a "User Calibration Mode" that nobody has worked out
  how to trigger, and some logging on the hardware UART.

## Disassembly notes

- Four small Philips screws inside the battery compartment, then pry the case
  clips with a small flathead.
- The PCB is retained by clips; the touchpad and trigger flex traces are stuck
  down with double-sided LCD tape, so expect to re-tape on reassembly.
- Enough internal volume that people have successfully transplanted the whole
  PCB into Nerf-gun shells for light-gun projects.

## Sources

See `references.md`. Primary: Samsung datasheet + user manual, FCC ID A3LETYO324,
Renesas DA14681 product page, and three independent teardowns
(jsyang, cwig, nullstalgia).
