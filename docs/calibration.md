# Magnetometer calibration

The AK09912 is mounted on the touchpad board, rotated relative to the IMU, and
its raw field has a large unit-specific hard-iron offset. On the first unit the
field sits near `(3240, -7845, -1240)` counts; the controller reports zero for
the first ~10 packets after streaming starts. Fusing that raw field makes yaw
worse, so MARG is opt-in and requires a validated profile.

## 1. Capture a tumble

```sh
gearvr-cli capture 'Gear VR' --seconds 60 -o tumble.jsonl
```

Rotate the controller slowly through as many orientations as you can — at least
one full turn in each axis, ideally a figure-eight — while keeping it away from
metal and magnets. The fit needs coverage on all three axes; a flat capture is
rejected.

## 2. Fit a profile

```sh
gearvr-cli mag-fit tumble.jsonl --out mag-calibration.toml
```

The fitter uses the min/max method: the centre of each axis extent is the
hard-iron offset, and each axis is scaled to the mean radius. It prints the
centre, axis radii, diagonal soft iron, and the residual (relative standard
deviation of the corrected field magnitude). A small residual means the field
looks spherical after correction; a large one usually means poor coverage.

Options:

- `--board-rotation <DEG>`: rotation from the magnetometer board to the
  controller frame. The documented AK09912 value is about `-30`; the default.
  This only shifts the heading reference, so drift correction works even if the
  angle is slightly off.
- `--min-samples <N>`: default 100.
- `--min-range <COUNTS>`: minimum raw spread per axis, default 100.

The command writes the profile only after the fit succeeds.

## 3. Run with MARG

```sh
gearvr-cli run 'Gear VR' --mag-calibration mag-calibration.toml --marg
```

MARG is disabled by default. Without `--marg`, the raw magnetometer is still
reported in JSON/state output but the 6-DOF orientation is unchanged. With it,
the calibrated field applies a gentle **yaw-only** correction: pitch and roll
stay with Madgwick, and a physical yaw is still tracked by the gyro. The world
heading is anchored at the first magnetometer sample after startup.

`--marg` without `--mag-calibration` is an error. A corrupt, unvalidated, or
version-mismatched profile is rejected at load time; the driver exits with the
parse/validation error rather than silently fusing bad data.

## File format

```toml
version = 1
hard_iron = [3240.0, -7845.0, -1240.0]
soft_iron = [[1.0, 0.0, 0.0], [0.0, 0.95, 0.0], [0.0, 0.0, 1.05]]
board_rotation_deg = -30.0

[meta]
created_unix = 1792536000
samples = 1200
residual = 0.02
validated = true
```

`validated = true` is required. `soft_iron` must be finite, symmetric, and
invertible; `meta.samples` and `meta.residual` must be present.

## Limitations

- The fit assumes the field magnitude is constant and the tumble covers all
  orientations. It does not model cross-axis soft iron.
- The scale is inherited from the `~0.06 µT/LSB` figure and has not been
  independently derived.
- The board rotation is a documented estimate, not measured per unit.
- A large hard-iron offset means a small tumble can produce a biased fit; check
  the residual and repeat the capture if it is high.
