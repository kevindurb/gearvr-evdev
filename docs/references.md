# References

All sources consulted, grouped by type. Access date for online sources: 2026-10-18.

## Primary sources (Samsung / component vendors / regulators)

| Source | URL | What it gives |
| --- | --- | --- |
| Samsung Gear VR Controller datasheet (retail/B2B PDF) | https://www.content.shi.com/SHIcom/ContentAttachmentImages/SharedResources/PDFs/Samsung/Gear%20VR%20Controller%20-%20Datasheet.pdf | Dimensions, weight, sensors list, BT 4.2 + claims of "HID Gamepad profile", battery, part numbers |
| Samsung ET-YO324 user manual Rev 1.1 (04/2017) | https://content.syndigo.com/asset/2e35c6e9-4cd9-45f3-b07e-d599f85a90e1/original.pdf | Button functions, pairing procedure, LED codes, sleep behaviour, PIN semantics, "no power on/off function" |
| FCC ID `A3LETYO324` | https://fccid.io/A3LETYO324 | Frequency range, conducted output, grant notes, exhibit list |
| FCC internal photos | https://fccid.io/A3LETYO324/Internal-Photos/Internal-Photos-ET-YO324-3288707 | Board photos (5 pages) |
| Renesas DA14681 product page | https://www.renesas.com/en/products/wireless-connectivity/bluetooth-low-energy/da14681-smartbond-bluetooth-low-energy-42-system-chip-soc | MCU specs: Cortex-M0, BLE 4.2, 64 kB OTP, 128 kB SRAM, 16 kB cache, 128 kB ROM, PMU, crypto |
| Samsung support page, ET-YO324BBEGJP | https://www.samsung.com/jp/support/model/ET-YO324BBEGJP/ | Regional SKU confirmation |
| Amazon listing, ET-YO324BBEGUS | https://www.amazon.com/Samsung-ET-YO324BBEGUS-Gear-VR-Controller/dp/B06XHXRXP1 | Current pricing/availability, user-reported quirks |

**Note on Samsung's own claims:** the datasheet says "BT 4.2 BLE + **HID Gamepad
profile**". This is misleading. The controller has a HID-over-GATT-shaped service
at a non-standard UUID whose report map declares only a keyboard, a 1-byte vendor
report and a consumer-control report — not a gamepad. This single line in a
marketing PDF is responsible for a lot of wasted effort in the community.

## Reverse engineering — protocol

| Source | URL | Notes |
| --- | --- | --- |
| Jim Yang, "Gear VR Controller Reverse Engineering" (2018-02-02) | https://jsyang.ca/hacks/gear-vr-rev-eng/ | The original work. Decompiled `com.samsung.android.app.vr.input.service`, mapped the GATT table, found the `Oculus Threemote` service, enumerated commands, first packet parser, teardown photos. **Contains the mag-at-offset-32 bug.** |
| jsyang/gearvr-controller-webbluetooth | https://github.com/jsyang/gearvr-controller-webbluetooth | Reference JS implementation + 3D demo |
| nullstalgia, "Reference — GearVR (Gochugaru)" (2025) | https://github.com/nullstalgia/daydream-catcher/blob/main/Writeups/Reference%20-%20GearVR%20(Gochugaru).md | Most complete GATT/record documentation, all 9 SUOTA char UUIDs, settings+sellout record layouts, 30 s connection-parameter disconnect behaviour |
| nullstalgia, "A brief peek inside GearVR's Controller" | https://github.com/nullstalgia/daydream-catcher/blob/main/Writeups/2.%20A%20brief%20peek%20inside%20GearVR%27s%20Controller.md | Firmware/SUOTA analysis, unsigned payloads confirmed, connection parameter states, LPM zero-sample bug |
| dennisonbertram/gearvr-controller, `PROTOCOL.md` (2025) | https://github.com/dennisonbertram/gearvr-controller/blob/main/PROTOCOL.md | **Hardware-verified.** The `0800`→ack→`0100` handshake, verified axis convention against gravity, 68 pkt/s and ~206 Hz figures, battery byte, touchpad lift marker, HID consumer channel, `0500` real serial |
| cwig/gearvr-exploration (2021) | https://github.com/cwig/gearvr-exploration | DA14681 + GD25LQ32 identification, debug pads and pinout, SUOTA flashing procedure, **magnetometer on touchpad board + −30° correction + offset 48** |
| polygraphene/FreePIEVRController | https://github.com/polygraphene/FreePIEVRController | First to process all three IMU samples; correct button bits; **sends `0800`+`0100` back to back (bug)** |
| gb2111/Access-GearVR-Controller-from-PC | https://github.com/gb2111/Access-GearVR-Controller-from-PC | C# port of jsyang; source of the swapped volume bits and 8-bit touchpad bugs |
| Samsung `com.samsung.android.app.vr.input.service` APK (decompiled) | — | Original source of truth: command classes, bit masks, `ui/c.class` debug strings, X.509 certs for `samsung`/`oculus` update signing, firmware assets |

## Reverse engineering — hardware

| Source | URL | Notes |
| --- | --- | --- |
| Cubits, "Gear VR Blaster — Part 1" (2017-05-27) | http://thinkcubits.blogspot.com/2017/05/gear-vr-blaster-part-1.html | Practical teardown: four Philips screws, case clips, flex traces on LCD tape, enough room to transplant the PCB |
| Hackaday coverage (2018-02-27) | https://hackaday.com/2018/02/27/reverse-engineering-opens-up-the-samsung-gear-vr-controller/ | Summary of jsyang's work |
| Hacker News discussion (2018-02-26) | https://news.ycombinator.com/item?id=16464299 | Context, minor corrections |

## Existing drivers / integrations

| Source | URL | Notes |
| --- | --- | --- |
| sameer/gearvr-controller-uhid | https://github.com/sameer/gearvr-controller-uhid | Rust, BlueZ + uhid, mouse output, Apache-2.0 |
| rdady/gear-vr-controller-linux | https://github.com/rdady/gear-vr-controller-linux | Python, pygatt + uinput, mouse |
| rdady/gear-vr-controller-windows | https://github.com/rdady/gear-vr-controller-windows | C#/LINQPad |
| rdady issue #1, "Possibility of creating a Kernel module?" | https://github.com/rdady/gear-vr-controller-linux/issues/1 | Open since 2019; never resolved |
| uutzinger/gearVRC | https://github.com/uutzinger/gearVRC | Python, bleak + AHRS + ZMQ/serial/viewer. Best fusion work. Documents `-vr` as not working |
| ilyabru/GearVrController4Windows | https://github.com/ilyabru/GearVrController4Windows | C# |
| MadTinker/gluetooth + GearVr_Plugin | https://github.com/MadTinker/gluetooth | GATT explorer; inherited the offset-32 mag bug |
| kaminoer/Dream-Mouse | https://github.com/kaminoer/Dream-Mouse | Windows/Android air mouse; Linux experimental/WIP |
| Tinnci/gear_vr_controller | https://github.com/Tinnci/gear_vr_controller | Rust + WinUI3, Windows-only, polished |
| jeong-jimin-github/psvr-steamvr | https://github.com/jeong-jimin-github/psvr-steamvr | Experimental SteamVR driver with GearVR BLE integration |
| vladoshub/xr-gate | https://github.com/vladoshub/xr-gate | Provider abstraction, experimental |
| kozec/sc-controller issue #399 | https://github.com/kozec/sc-controller/issues/399 | Request for support; never implemented |
| ShimuraWorkshop, "Gear VR Controller Motion Pointer for Windows" | https://shimuraworkshop.itch.io/gear-vr-controller-motion-pointer-for-windows | Closed-source Windows tool |

## Linux input / integration

| Source | URL | Notes |
| --- | --- | --- |
| ABeltramo, "Creating a Virtual DualSense Controller via UHID" (2024-03-24) | https://abeltra.me/blog/inputtino-uhid-2/ | First post: why uinput cannot expose DualSense features |
| ABeltramo, "Beyond USB: Improving Virtual Controller Support in Linux Games" (2025-01-29) | https://abeltra.me/blog/inputtino-uhid-3/ | **Key.** `BUS_BLUETOOTH` uhid works where `BUS_USB` does not; hidapi parent-USB check; CRC32; 16 ms report timing |
| games-on-whales/inputtino | https://github.com/games-on-whales/inputtino | Reference implementation of virtual devices |
| nick.tay.blue, "Wine + proper DualSense support via hidraw" (2024-01-21) | https://nick.tay.blue/2024/01/21/wine-dualsense/ | How hidraw-based DS support differs from evdev |
| ValveSoftware/steam-devices `60-steam-input.rules` | https://github.com/ValveSoftware/steam-devices/blob/master/60-steam-input.rules | udev rules Steam expects |
| Steamworks docs, "Input Sources" | https://partner.steamgames.com/doc/features/steam_controller/input_source | **Gyro is officially available only on Steam Controller, PS4 and DualSense.** No gyro input source for generic pads or Xbox pads |
| ValveSoftware/steam-for-linux issue #13394 | https://github.com/ValveSoftware/steam-for-linux/issues/13394 | **Open regression:** generic evdev/DirectInput gamepads enumerated but not opened by Steam Input after udev 259 (Fedora 44, also Ubuntu 24.04); HIDAPI unaffected |
| ValveSoftware/steam-for-linux issue #8862 | https://github.com/ValveSoftware/steam-for-linux/issues/8862 | Switch Pro gyro on Linux |
| Monado controller/evdev drivers | https://monado.pages.freedesktop.org/monado/group__drv.html | OpenXR runtime; evdev controller driver |

## Component datasheets (for later)

| Part | Where |
| --- | --- |
| DA14681 | https://www.renesas.com/en/document/dst/da14681-datasheet |
| GD25LQ32 | GigaDevice 25-series SPI NOR datasheet |
| AK09912 | Asahi Kasei AK09912 magnetometer datasheet |
| LSM6DS3 / LSM6DSL | ST 6-axis IMU datasheets (sensitivity confirms the family, not the part) |

## Conflicts recorded in these docs

Where sources disagree, the docs flag it inline. Summary:

1. **Magnetometer byte offset** — 32 (jsyang, wrong) vs 48 (everyone later, correct).
2. **IMU sample handling** — 1 per packet (jsyang, gb2111) vs 3 per packet (polygraphene, everyone later, correct).
3. **Command handshake** — `0100` alone (~30 pkt/s) vs `0800`→ack→`0100` (68 pkt/s). Only dennisonbertram gets it right.
4. **Volume up/down bit order** — swapped in gb2111's port.
5. **"13-bit" accel/gyro** (nullstalgia's table) vs full int16 (empirically correct).
6. **Axis convention** — dennisonbertram's gravity-verified convention vs uutzinger's permute. Unresolved; see `open-questions.md`.
7. **`-vr` flag usefulness** — documented as broken by uutzinger, but the handshake is what actually matters, not the mode command alone.
8. **Samsung's "HID Gamepad profile" claim** — false for practical purposes.
9. **Whether HID exists at all** — yes, at a non-standard UUID, but with no usable gamepad report.
