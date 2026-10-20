# Packet formats

Notifications arrive on `c8c51726-81bc-483b-a052-f7a14ea3d281`. They are of
**variable length, and the length identifies the type.** All multi-byte fields are
**little-endian** unless stated otherwise.

| Length | Type |
| --- | --- |
| 2 | Command acknowledgement (echo of the command) |
| `12 + 16*N` | **Sensor packet** — N IMU samples (usually 3) + 12-byte suffix |
| 40 | Device settings |
| 66 | Calibration record |
| 68 | Device settings + "sellout" (model/serial) record |

A sensor packet at the steady state is therefore `12 + 48 = **60 bytes**`.
Your parser must dispatch on length, not assume 60.

---

## Sensor packet — IMU sample (16 bytes each)

N samples, **oldest to newest**, packed back to back from offset 0.

| Offset | Size | Field | Notes |
| --- | --- | --- | --- |
| +0 | u32 | timestamp | **microseconds**, device clock |
| +4 | i16 | accel X | |
| +6 | i16 | accel Y | |
| +8 | i16 | accel Z | |
| +10 | i16 | gyro X | |
| +12 | i16 | gyro Y | |
| +14 | i16 | gyro Z | |

### Scaling

| Quantity | Raw units | Conversion | Full scale |
| --- | --- | --- | --- |
| Accelerometer | 2048 LSB/g | `g = raw / 2048`; `m/s² = raw * 9.80665 / 2048` = `raw * 0.0047884033` | ±16 g |
| Gyroscope | 14.285 LSB/(°/s) | `°/s = raw / 14.285`; `rad/s = raw * 0.017453292 / 14.285` = `raw * 0.0012217915` | ±2000 °/s |
| Magnetometer | ~0.06 µT/LSB (unconfirmed) | `µT = raw * 0.06` | — |

Notes:

- The 14.285 figure is `10000 / 700` — the classic ST `±2000 dps → 70 mdps/LSB`
  setting. Independent integration of the gyro between static poses fitted
  **14.40**, ~1% off, which is within the noise of that experiment. Treat 14.285
  as correct but be aware a per-unit scale factor may be worth calibrating.
- Typical gyro zero-rate bias is around **(−1.1, −4.5, +1.8) °/s**. This is large
  enough to matter; compensate or you will get visible yaw drift in seconds.
- jsyang's factor of `10000.0 * 9.80665 / 2048.0` then multiplied by `1e-5` is
  algebraically the same as `9.80665/2048`; the `1e4`/`1e-5` pair is a leftover
  from float precision juggling. Don't copy it verbatim.
- **Conflict:** one writeup describes accel/gyro as "13-bit signed integers".
  The empirical fits (|a| ≈ 2030–2100 at rest, consistent with 1 g at 2048 LSB/g)
  only work if you treat them as full 16-bit two's complement. Parse as i16.

### Sample rate

Three samples per 60-byte packet at ~68 packets/s ⇒ **~206 Hz** (Δt ≈ 4850 µs).
The timestamps confirm this. At the slow (non-VR) rate you get far fewer packets
and correspondingly sparse samples.

### Axis convention

Right-handed, per hardware-verified measurement against gravity in known poses:

- **+X = right**
- **+Y = forward** (toward the far / trigger end — the direction you point)
- **+Z = out of the touchpad face** (toward the user's thumb)

Gyro uses the same axes with the right-hand rule: turning left gives **+Z**,
raising the nose gives **+X**.

**This conflicts with other writeups.** uutzinger's gearVRC documents raw axes as
"acc.x points left, acc.y points toward user, acc.z down" and applies the permute
`acc −Y−X+Z`, `mag −X+Y+Z`, `gyr +Y+X−Z` to get to an x-forward/y-east/z-down
frame. That is a *different* frame choice, not necessarily a contradiction — but
if your fusion output is rotated 90° from what you expect, this is why. Pick one
convention, document it, and test it against gravity.

The touchpad board's ~30° rotation relative to the IMU (and the mag's inverted X)
must be corrected before fusing the magnetometer. See `open-questions.md`.

---

## Sensor packet — suffix (12 bytes, starts at offset `16*N`)

For the common N=3 case that is offset 48.

| Offset (N=3) | Size | Field | Notes |
| --- | --- | --- | --- |
| 48 | i16 | magnetometer X | raw; ~0 for the first second after streaming starts |
| 50 | i16 | magnetometer Y | |
| 52 | i16 | magnetometer Z | |
| 54 | u8 | touchpad status + X high bits | `0b00SSxxxx` |
| 55 | u8 | touchpad X low bits + Y high bits | `xxxxxxyy` |
| 56 | u8 | touchpad Y low bits | `yyyyyyyy` |
| 57 | u8 | temperature | °C, integer. Rises ~25 → 28 while held. |
| 58 | u8 | buttons | see bit table |
| 59 | u8 | battery | percent 0–100 |

### Magnetometer

- AK09912, on the touchpad board.
- Raw int16 with a **large hard-iron offset** (X ≈ +6000 observed). Scale
  (~0.06 µT/LSB) and axis alignment are **not** firmly established.
- jsyang's original code read the magnetometer at offset **32** — that is wrong.
  Offset 32 is sample 3's timestamp and accelerometer. The correct offset is 48.
  This was reported in his issue tracker and is still wrong in his published demo,
  so anyone who forked his code inherited the bug (check: several forks do).
- **Unusable without calibration.** Normalize min/max per axis (a figure-8 swing
  while holding Back+Trigger is the convention used by cwig's build), then apply
  the −30° board rotation.

### Touchpad

```
byte 54:  0  0  S  S  x  x  x  x
byte 55:  x  x  x  x  x  x  y  y
byte 56:  y  y  y  y  y  y  y  y

x = ((b54 & 0x0F) << 6) | (b55 >> 2)      10-bit, 0 = left      .. ~315 = right
y = ((b55 & 0x03) << 8) | b56             10-bit, 0 = far edge  .. ~315 = near edge
```

`SS` (bits 5:4 of byte 54):

| `SS` | Meaning |
| --- | --- |
| `01` | finger touching |
| `10` | not touching (idle) |
| `00` | finger **just lifted** — sent for exactly one packet |

That one-packet `00` lift marker is genuinely useful: it gives you clean tap
detection without a timeout heuristic. Idle reads as `20 00 00`.

- The pad is circular and physically ~36 mm across, but the coordinate space is
  square: both axes span roughly **7–314** around the rim, with a reported
  full-scale of **320** (`0x140`) per the `0500` device record.
- Coordinates are **10-bit fields packed big-endian-ish across byte boundaries** —
  X's high bits come from byte 54, Y's from byte 55. Getting this wrong silently
  produces plausible-looking-but-wrong values, which is why two implementations
  (gb2111's C# port, and one ESP32 attempt) ended up with swapped or 8-bit axes.
- The `0500` settings record also exposes "touch size" fields (320/320) which one
  writeup hoped were physical dimensions. They are not; they are the logical range.

### Temperature

Offset 57, integer °C. Useful as a sanity check and not much else. Note that
using it for gyro bias temperature compensation is theoretically attractive but
nobody has characterized the relationship.

### Buttons (byte 58)

| Bit | Mask | Button |
| --- | --- | --- |
| 0 | `0x01` | trigger |
| 1 | `0x02` | home |
| 2 | `0x04` | back |
| 3 | `0x08` | touchpad click |
| 4 | `0x10` | volume up |
| 5 | `0x20` | volume down |
| 6 | `0x40` | **idle flag** — clear while any button is down, set ~20 ms after release |
| 7 | `0x80` | unused |

- Bit 6 is an edge-assist from the firmware; you can ignore it and use your own
  edge detection, but it is handy for debouncing "all released".
- **gb2111's C# port has volume up/down swapped.** jsyang, polygraphene and
  nullstalgia all agree on the ordering above.

### Battery (byte 59)

Whole percent. Also available from the standard `0x180F`/`0x2A19` characteristic
(read + notify). Prefer the standard characteristic for a UI gauge and treat this
byte as a cross-check; they have been observed to agree.

---

## Command acknowledgement (2 bytes)

The controller echoes an applied command back on the data characteristic as the
same two bytes. Not every command is acked:

| Acked | Not acked |
| --- | --- |
| `06 00` LPM enable | `00 00` off |
| `07 00` LPM disable | `01 00` sensor mode (see note) |
| `08 00` VR mode (~1.5 s delay) | `02 00` HID |
| | `04 00` keep-alive |

Note on `01 00`: in the broken ordering (`08 00` then `01 00` immediately) the
controller *does* eventually echo `01 00` — right before the stream dies. So
treating "saw `01 00` ack" as success would be wrong. Only `08 00` (and `06`/`07`)
are meaningful.

---

## Device settings record (40 bytes, or 68 with the serial suffix)

Reply to command `05 00`. Ten uint32 fields, then optionally a 28-byte suffix.

| Offset | Size | Field | Observed value |
| --- | --- | --- | --- |
| 0 | u32 | IMU sample interval | `05 00 00 00` = 5 (ms) |
| 4 | u32 | Magnetometer sample interval | `0a 00 00 00` = 10 (ms) |
| 8 | u32 | Hardware ID | `01 00 00 00` |
| 12 | 4 bytes | Firmware revision (ASCII-ish) | `"QC1\0"` on fw `YO324XXU0AQC1`; numeric `3425361` reported for `AQD4` |
| 16 | u32 | Button mask | `3f 00 00 00` = 63 = `0b111111` (six buttons — matches byte 58 bits 0–5) |
| 20 | u32 | Touch X max | `40 01 00 00` = 320 |
| 24 | u32 | Touch Y max | 320 |
| 28 | u32 | Touch X size | 320 |
| 32 | u32 | Touch Y size | 320 |
| 36 | u32 | ? | `01 00 00 00` |

Optional suffix at offset 40 (present on retail units, hence 68-byte replies):

| Offset | Size | Field |
| --- | --- | --- |
| 40 | 8 | Model name, NUL-padded ASCII — `"ET-YO324"` |
| 48 | 20 | **Real serial number**, NUL-padded ASCII — e.g. `"ET-YO324RF7J416JM1Z"` |

The Device Information service reports a placeholder serial of `123456`. The
`0500` record gives you the actual unit serial, which is the only way to
distinguish controllers if you own several.

**The 5 ms IMU interval is interesting**: it implies the firmware samples at
~200 Hz internally, matching the observed ~206 Hz aggregate rate.

---

## Calibration record (66 bytes) — UNDECODED

Reply to command `03 00`. **Byte-for-byte identical on every read** from a given
unit, high entropy, and does not decode as int16 or float32 at any obvious
stride.

Two independent parties describe it as eight packed vec3 entries plus a
temperature:

| Offset | Size | Field (speculative) |
| --- | --- | --- |
| 0 | 8 | accelerometer bias vector |
| 8 | 8 | gyroscope bias vector |
| 16 | 8 | accelerometer alignment/scale row X |
| 24 | 8 | row Y |
| 32 | 8 | row Z |
| 40 | 8 | gyroscope alignment/scale row X |
| 48 | 8 | row Y |
| 56 | 8 | row Z |
| 64 | 2 | temperature at calibration time (centi-°C, LE: `2500` = 25.00 °C) |

Each 8-byte vec3 entry is claimed to be **big-endian 21-bit-per-axis** with one
padding bit:

| Bits | Field |
| --- | --- |
| 21 | X |
| 21 | Y |
| 21 | Z |
| 1 | padding, always 0 |

That is 64 bits exactly, which is a plausible packing. But **nobody has applied
this record to raw sensor data and shown that it improves anything.** The Samsung
APK calls these "bias" values. If you decode this and demonstrate that applying it
reduces drift, that is a genuinely new result and the most valuable single
contribution available in this space — see `open-questions.md`.

**First real unit (2026-10-20, `2C:BA:BA:4F:00:14`, firmware `QD4`):** bit 0 is
the only bit that is zero in all eight groups, confirming the padding bit is the
LSB. The tail is `2500`, i.e. 25.00 °C. However, decoding groups 2–7 as two 3×3
matrices yields no plausible alignment, so the semantic layout is still
unresolved.

---

## Packet parser pseudo-code

```
on_notify(data):
    match len(data):
        2      -> command_ack(u16_le(data))
        40, 68 -> settings(data)
        66     -> calibration(data)
        n where (n - 12) % 16 == 0 and n >= 12:
                   n_samples = (n - 12) / 16
                   samples = [parse_imu(data[16*i .. 16*i+16]) for i in range(n_samples)]
                   suffix  = parse_suffix(data[16*n_samples .. ])
        else   -> log_unknown(data)
```

Two edge cases that will bite you:

1. **Zero samples.** A known firmware bug produces `12 + 0` = 12-byte sensor
   packets (valid suffix, no IMU). If `07 00` is sent as the first command after
   wake without `01 00`, this is what you get. Your `(n-12) % 16 == 0` branch
   handles it, but downstream code must not assume at least one sample exists.
2. **Truncated notifications.** Some hosts deliver the first notification before
   MTU negotiation settles. If your stack hands you fewer than 60 bytes at the
   start of a stream, drop it rather than misparsing.

## Sources

Byte layouts cross-checked across jsyang (2018), polygraphene's FreePIE plugin,
nullstalgia's Gochugaru reference (2025, most detailed on records), and
dennisonbertram's hardware-verified PROTOCOL.md (2025, most detailed on the
handshake and verified axis conventions).
