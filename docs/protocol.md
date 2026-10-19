# Protocol

All of this is verified on hardware by at least one independent party, and most
of it by three or more. Where sources disagree it is flagged.

## Advertising

| Item | Value |
| --- | --- |
| Local name | `Gear VR Controller(XXXX)` |
| `XXXX` | First two bytes of the BLE address, **reversed**, uppercase hex. Address `2C:BA:BA:2E:17:DB` → `Gear VR Controller(17DB)`. |
| Appearance | `0x03C0` (960) — HID Generic |
| Manufacturer data | Company ID `0x0075` (Samsung) |
| Manufacturer payload | `01 00 02 00 FB 01 02 0E 03` then ASCII `vrsetupwizard` then `10` |

Two useful consequences:

- **You can derive the MAC from the advertised name**, or vice versa. Handy for
  matching a known controller without a scan-and-guess.
- The name suffix is a stable per-unit identifier for UX ("Connect to (17DB)").

## GATT table

| Service | Char | Properties | Contents |
| --- | --- | --- | --- |
| `0x1800` Generic Access | `2A00` `2A01` `2A04` | read | name, appearance, preferred connection params |
| `0x1801` Generic Attribute | — | — | empty |
| `0x180A` Device Information | `2A29` `2A24` `2A25` `2A26` `2A27` `2A28` `2A50` | read | "Samsung", "ET-YO324", serial `123456` (placeholder!), fw, hw `Rev0.0`, sw = firmware version string, PnP ID |
| `0x180F` Battery | `2A19` | read, notify | whole percent 0–100 |
| `0x1879` (non-standard UUID, HID-over-GATT layout) | `2A4E` `2A4A` `2A4B` `2A4C` `2A4D` `2A22` `2A32` | mixed | see below |
| `0xFEF5` Dialog SUOTA | 9 chars | mixed | firmware update; leave alone |
| `4f63756c-7573-2054-6872-65656d6f7465` | `…d281` | read, notify | **sensor data** |
| (vendor service) | `…d282` | read, write | **commands** |

### The vendor service UUID

```
4f63756c-7573-2054-6872-65656d6f7465
```

ASCII-decoding the first 16 bytes gives `Oculus Threemote`. (Note the typo in the
original: "Three" lost an `e` when the ASCII ran out at 16 bytes.) This is
Oculus/Samsung's internal name for the Gear VR remote — the hardware codename
appears to have been "Gochugaru", a name chosen by a reverse engineer rather than
Samsung, so don't rely on it.

Characteristics:

- `c8c51726-81bc-483b-a052-f7a14ea3d281` — **data receive**, notify. Sensor
  packets, command acknowledgements, and record replies all arrive here.
- `c8c51726-81bc-483b-a052-f7a14ea3d282` — **command send**, write. Reading it
  returns the last command written; nothing more.

### The 0x1879 service — HID-ish but useless for us

This is a HID-over-GATT service with a **non-standard service UUID** and a report
map that only declares:

- report 1: keyboard (8 modifier bits + 6 keycodes + 5 LED output bits)
- report 2: vendor-defined, 1 byte, logical 1–100
- report 3: consumer control, 16-bit usage

The only thing ever observed on `2A4D` is `EA 00` (Volume Decrement) on press and
`00 00` on release. Reports 1 and 2 have never been seen in use.

**Reading or subscribing to any characteristic in this service requires
encryption, which triggers bonding.** That is actually useful: it is one way to
force the controller to pair. The vendor service works fine *without* bonding.

### Consequences for driver design

The IMU, trigger, and touchpad are **not** reachable through HID. There is no
report descriptor that carries them. This is the single reason no kernel `hid-*`
driver exists for this device — see `linux-integration.md`.

## Commands

Two bytes, written to `…d282`. **Little-endian** in the sense that the first byte
is the command ID and the second is a parameter that is always `00`.

| Bytes | Name | Reply | Effect |
| --- | --- | --- | --- |
| `00 00` | Off | none | Stops streaming. |
| `01 00` | Sensor mode | sensor packets | Enables notifications at the *current* connection interval. |
| `02 00` | HID mode | none | Sets connection interval to `72` (90 ms/packet) and stops streaming. Presumably for Gear 360 camera shutter use. |
| `03 00` | Calibration record | 66-byte blob | Stops streaming, replies with a constant high-entropy 66-byte record. Undecoded. |
| `04 00` | Keep-alive | none | Originally sent every ~10 s. **Does not prevent sleep-disconnect.** |
| `05 00` | Device record | 40-byte or 68-byte packet | Stops streaming, replies with sample intervals, HW/FW IDs, button mask, touchpad range, and (in the 68-byte form) model + real serial. |
| `06 00` | LPM enable | `06 00` | Low-power mode: sensors off, interval `72`, stays connected. Sent when the phone screen turns off in a Gear VR. |
| `07 00` | LPM disable | `07 00` | Interval `11` (13.75 ms), streaming resumes. |
| `08 00` | VR mode | `08 00` after ~1.5 s | Interval `11` (13.75 ms), high rate. Stops any current stream. |
| anything else | — | ignored | No ack, no reply; the stream stops. Confirmed ignored: `09 00`, `0a 00`, `0f 00`, `01 01`, `08 01`, `0b 00`–`0e 00`, `10 00`. |

**There is no `0x09`/higher command hiding a faster mode.** The `08 00` VR mode is
the fast one.

## The command acknowledgement handshake (critical)

The controller sends a **2-byte notification echoing an applied command** on
`…d281`. This is the most commonly-botched part of the protocol.

Ordering matters:

| Sequence | Result |
| --- | --- |
| `08 00` → wait for `08 00` ack → `01 00` | **68 packets/s, 3 IMU samples each (~206 Hz), steady indefinitely.** This is the correct sequence. |
| `08 00` then `01 00` immediately | Streams fast for ~1.5 s, then echoes `01 00` and **stops**. |
| `01 00` alone | Slow: ~30 pkt/s with gaps. |
| `01 00` then `08 00` | Streaming stops immediately; `08 00` acked ~1.5 s later. |

The ~1.5 s delay on the `08 00` ack is almost certainly a connection-parameter
update round-trip.

**Prior art that gets this wrong:**

- polygraphene's FreePIE plugin writes `0800` and `0100` back to back without
  waiting — kills the stream after ~1.5 s on at least one unit.
- gb2111's C# port only sends `0100` — slow rate.
- sameer's Rust driver sends only `0100` — slow rate.
- rdady's Python sends `0100` — slow rate.
- jsyang's original sends commands from UI buttons with no sequencing.
- uutzinger's gearVRC exposes a `-vr` flag for VR mode and documents it as
  **not working**.

So: **almost every existing driver runs at the slow rate.** Getting the
handshake right is a meaningful improvement over all of them.

## Connection parameters

The controller actively requests connection parameter changes and **will
disconnect from the host if it cannot negotiate them within 30 s.** Two target
states:

| Mode | Connection interval | Units | Bytes/packet rate |
| --- | --- | --- | --- |
| VR (fast) | `11` | 13.75 ms | ~68 pkt/s |
| HID (slow) | `72` | 90 ms | ~11 pkt/s nominal |

The device's own Preferred Connection Parameters (`2A04`) read as 13.75 ms
min/max, slave latency 0, supervision timeout 10000 ms.

Practical implication: your host's BlueZ must permit the parameter update. Most
adapters do; some USB dongles and some `bluetoothd` configs will fight it.

## Pairing and bonding

This is the most fragile part of the whole device and where driver UX matters most.

- The controller **requires bonding** before the vendor service will deliver
  data. The `0x1879` HID chars force encryption explicitly; `…d281`/`…d282` work
  without it, but you still want a bond so the controller can reconnect to you.
- PIN is the four-digit number shown in the pairing prompt; it differs per unit
  and is shown to the user as `Gear VR Controller(0000)`-ish in Android's UI.
- **Holding Home for a few seconds enters pairing mode and erases the existing
  bond.** This is also the "I'm hopelessly wedged, start over" button.
- Known failure modes:
  - Toggling rapidly between paired/unpaired after a sleep/wake cycle
    (documented by jsyang in 2018).
  - Controller enters a pairing state where it **refuses all pairing attempts**
    from the host and stays there. On Linux, a reboot sometimes clears it; the
    reliable fix is to remove the device entirely (`bluetoothctl remove <MAC>`),
    pull the batteries, and start over.
  - macOS specifics: after a Home long-press wipes the bond, CoreBluetooth fails
    with `CBErrorDomain 14 "Peer removed pairing information"` until you
    *Forget This Device*; `blueutil --unpair` does not work on current macOS.
  - Windows: needs the device removed from Settings, and sometimes twice, because
    the list does not clear.

### BlueZ recipe that works

```
power on
agent on
default-agent
scan on                     # wait for Gear VR Controller(XXXX)
pair <MAC>                  # accept the PIN prompt
trust <MAC>                 # REQUIRED, or the controller won't auto-reconnect
connect <MAC>
```

`trust` is the step people miss. Without it you get a bond that never
auto-reconnects.

Healthy state looks like:

```
Device <MAC> (public)
    Name: Gear VR Controller(17DB)
    Paired: yes
    Bonded: yes
    Trusted: yes
    UUID: Generic Access Profile    (00001800-0000-1000-8000-00805f9b34fb)
    UUID: Generic Attribute Profile (00001801-0000-1000-8000-00805f9b34fb)
    UUID: Device Information        (0000180a-0000-1000-8000-00805f9b34fb)
    UUID: Battery Service           (0000180f-0000-1000-8000-00805f9b34fb)
    UUID: Unknown                   (00001879-0000-1000-8000-00805f9b34fb)
    UUID: Dialog Semiconductor GmbH (0000fef5-0000-1000-8000-00805f9b34fb)
    UUID: Vendor specific           (4f63756c-7573-2054-6872-65656d6f7465)
```

### Sleep / wake behaviour to design around

- Idle → controller sleeps, drops the link.
- Any button press wakes it; it then attempts to reconnect to the last bonded host.
- Because the controller has no power switch, **the only way to force a clean
  restart is to remove the batteries.**
- Button activity keeps it awake; keep-alive commands do not.

## Known firmware quirk

If `07 00` (LPM disable / wake) is sent as the **first** command after power-on
or wake-from-deep-sleep, *without* `01 00` (sensor mode) first, you get
notifications containing a **zero-length IMU sample array** but a valid 12-byte
suffix (magnetometer, buttons, temperature). You have no device timestamps to
compute a delta against. Suspected firmware bug. Your parser must tolerate it.

## Sources

`protocol.md` is compiled from: jsyang (2018), nullstalgia's `Gochugaru` reference
(2025), dennisonbertram's hardware-verified `PROTOCOL.md` (2025), cwig (2021),
uutzinger's gearVRC (2023). See `references.md`.
