# Gear VR Controller — research index

Collected findings on the Samsung Gear VR Controller (model **ET-YO324**,
US SKU `ET-YO324BBEGUS`), assembled from teardowns, FCC filings, Samsung's own
Android service APK, and several independent reverse-engineering efforts.

## Why this device is a good driver target

- Cheap and abundant secondhand (~$10–14 on Amazon, less on eBay/Poshmark).
- **Fully mapped protocol.** You are writing a driver, not doing archaeology.
- Real physical inputs: clickable capacitive touchpad, index trigger, Home, Back,
  volume rocker — plus a 9-axis IMU.
- AAA batteries (no charging state machine to babysit), ~40 g, pocketable.

## Why it is a bad driver target

- **Not HID.** The IMU/trigger/touchpad data only comes out of a vendor GATT
  service. There is no way to get it through the kernel's HID stack, which is why
  no in-tree driver exists and why a kernel module would be a Bluetooth GATT
  client driver rather than a `hid-*` driver.
- **Disconnects on inactivity**, by design, with no way to prevent it. Sleep is
  triggered by lack of button activity; keep-alives do not stop it.
- **No on-device sensor fusion.** You get raw accel/gyro/mag and must fuse.
- **Pairing is fragile** and can wedge into a state that refuses new pairings.
- Magnetometer is mounted on a separate PCB, rotated ~30° from the IMU axes.
- Some of the interesting protocol (the `0300` calibration record) is still
  undecoded by anyone.

## Reading order

1. [`hardware.md`](hardware.md) — what is actually inside.
2. [`protocol.md`](protocol.md) — how to talk to it.
3. [`packets.md`](packets.md) — how to parse what it says.
4. [`prior-art.md`](prior-art.md) — who has done this before, and their mistakes.
5. [`linux-integration.md`](linux-integration.md) — how to present it to Linux.
6. [`open-questions.md`](open-questions.md) — what is still unknown.
7. [`DESIGN.md`](DESIGN.md) — what we are going to build.
8. [`bringup.md`](bringup.md) — first-contact checklist for real hardware.
9. [`implementation.md`](implementation.md) — what is built and what was
   validated on a Steam Deck.
10. [`../packaging/README.md`](../packaging/README.md) — udev, pairing, and the
    systemd user unit.

## One-paragraph summary

A Dialog Semiconductor DA14681 Cortex-M0 BLE 4.2 SoC with a GD25LQ32 SPI flash,
an ST-family 6-axis IMU in the handle, an AK09912 magnetometer up on the
touchpad board, and a capacitive touchpad, powered by two AAA cells. It
advertises a vendor GATT service whose 16-bit UUID is literally the ASCII string
`Oculus Threemote`. Writing the two-byte command `0800`, waiting ~1.5 s for the
`08 00` acknowledgement, then writing `0100` yields a steady 68 notifications/s,
each carrying three 16-byte IMU samples (~206 Hz), a magnetometer reading,
touchpad coordinates, temperature, button bits and battery percentage.
