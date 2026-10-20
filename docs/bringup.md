# First-contact bring-up (ET-YO324)

The ordered checklist for the first physical controller. Steps map to the
hardware tasks in
[`openspec/changes/complete-remaining-driver-work/tasks.md`](../openspec/changes/complete-remaining-driver-work/tasks.md)
section 8. Record what actually happened; do not mark anything validated from
memory.

## 0. Host preparation

The reference test host is the Steam Deck running Bazzite in desktop mode. The
release binary is staged at `~/gearvr-probe/gearvr-cli`, and the udev rule is
already installed there.

On the development host:

```sh
cargo build --release -p gearvr-cli
scp target/release/gearvr-cli steamdeck:~/gearvr-probe/gearvr-cli
scp scripts/deck-capture.sh steamdeck:~/gearvr-probe/deck-capture.sh
```

On the Deck:

```sh
~/gearvr-probe/gearvr-cli doctor
sudo install -m 0644 packaging/udev/70-gearvr-evdev.rules /etc/udev/rules.d/
sudo udevadm control --reload-rules
sudo udevadm trigger --subsystem-match=input
```

`doctor` should report `uinput writable: true`, a powered adapter, and
`gearvr udev rule: installed`. `uhid writable: false` is expected and only
matters for the future DualSense backend.

`evtest`/`jstest` are not required to drive the controller. On an immutable
Fedora/Kinoite host they must be layered (`rpm-ostree install evtest`) and need
a reboot; `run --backend json` is a dependency-free substitute for a
quick stream check.

- [ ] release binary built
- [ ] `uinput writable: true`
- [ ] adapter powered
- [ ] udev rule installed

## 1. Power and pair

Insert AAA batteries / pull the tab. Hold **Home for about 4 seconds** until the
LED flashes: this enters pairing mode and erases any existing bond, including
one from a phone.

```text
bluetoothctl
power on
agent on
default-agent
scan on
# wait for: [NEW] Device AA:BB:CC:DD:EE:FF Gear VR Controller(XXXX)
pair AA:BB:CC:DD:EE:FF       # accept/confirm the PIN prompt
trust AA:BB:CC:DD:EE:FF      # required for reconnect after sleep
connect AA:BB:CC:DD:EE:FF
info AA:BB:CC:DD:EE:FF       # expect Paired: yes, Bonded: yes, Trusted: yes
```

Write down the MAC, the PIN prefix, the firmware string, and the date.

If pairing wedges or returns `AuthenticationFailed`:

```text
remove AA:BB:CC:DD:EE:FF
```

then pull the batteries for ten seconds, hold Home again, and retry. Do not
toggle connect/disconnect rapidly; that is a known way to wedge the bond.

- [ ] Paired: yes
- [ ] Bonded: yes
- [ ] Trusted: yes
- [ ] vendor GATT service listed in `info`

## 2. Confirm BlueZ sees it

```sh
~/gearvr-probe/gearvr-cli scan --seconds 8
```

The controller is marked with `*` when its name or vendor service UUID matches.

- [ ] device listed with paired/trusted state

## 3. Capture the handshake (task 8.1)

On the Deck, the staged helper runs steps 3 and 4 together:

```sh
~/gearvr-probe/deck-capture.sh 'Gear VR' 60
```

Or run them individually:

```sh
mkdir -p ~/gearvr-captures
~/gearvr-probe/gearvr-cli capture 'Gear VR' --seconds 90 -o ~/gearvr-captures/first.jsonl
```

`capture` creates no input device and needs no backend. It writes one JSON
object per line:

```json
{"v":1,"t_us":1523123,"dir":"rx","hex":"0800"}
```

`t_us` starts before the BLE connection is attempted, so connect and handshake
latency are measurable.

Expected traffic:

| when | dir | bytes | meaning |
| --- | --- | --- | --- |
| start | tx | `0800` | VR mode |
| +1.0–3.0 s | rx | `0800` | echo; never retry before 3 s |
| right after echo | tx | `0100` | sensor mode |
| every ~14.7 ms | rx | 60 bytes (`12 + 16*3`) | sensor packet |
| start | rx | 40 or 68 bytes | settings record |
| only after `0300` | rx | 66 bytes | calibration record |

Ctrl-C is safe: each record is flushed as it is written.

Inspect:

```sh
~/gearvr-probe/gearvr-cli replay ~/gearvr-captures/first.jsonl --include-tx --summary --limit 12
```

If the stream never starts, keep the whole capture and add the tx/rx ordering
and timings to `docs/implementation.md`. For pairing-level detail, capture HCI
traffic at the same time:

```sh
sudo btmon -w ~/gearvr-captures/hci.btsnoop
```

- [ ] `0800` echo captured
- [ ] `0100` sent only after the echo
- [ ] sensor packets at roughly 68/s
- [ ] no duplicate `0800` write inside one second

## 4. Read the constant records (task 8.2)

`probe` does not run the handshake, so it is safe before or after a session. It
reads the data characteristic once, then sends the requested command IDs and
listens for `--wait-ms` after each.

```sh
~/gearvr-probe/gearvr-cli probe 'Gear VR' -o ~/gearvr-captures/records.jsonl
# equivalent to --commands 03,05
```

- `03 00` returns the 66-byte calibration record and **stops streaming**.
- `05 00` returns the 40- or 68-byte settings/device record.

Replay it:

```sh
~/gearvr-probe/gearvr-cli replay ~/gearvr-captures/records.jsonl --include-tx
```

Keep the JSONL plus the exact bytes printed by `replay`; these become the first
real protocol fixtures (tasks 1.1 and 1.2).

- [ ] 66-byte calibration record captured
- [ ] 40- or 68-byte settings record captured
- [ ] firmware, model, and serial noted
- [ ] capture files named with the unit MAC and date

## 5. Stream into a real backend (task 8.1)

```sh
~/gearvr-probe/gearvr-cli run 'Gear VR' --backend json --duration 10
~/gearvr-probe/gearvr-cli run 'Gear VR' --backend evdev --motion --volume-buttons
```

Press trigger, Back, Home, and the touchpad click; drag the pad; tilt and move
the controller. Confirm the gamepad is `1209:0001`, the motion node `1209:0002`
(only with `--motion`), and that every held input releases when you stop.

- [ ] gamepad enumerated
- [ ] motion node enumerated and readable
- [ ] all four buttons plus touchpad click register
- [ ] touchpad coordinates span the full 0–320 range
- [ ] no stuck inputs

## 6. Axis convention (task 8.3)

Rest the controller flat, touchpad up, trigger pointing away from you. Record
about five seconds per pose with `capture`, then replay and read the raw accel:

1. flat and still — the dominant axis should read about +1 g
2. trigger end tilted down
3. trigger end tilted up
4. rolled 90° left, then right
5. yawed left, then right

The documented final frame is **+X right, +Y forward (toward the trigger), +Z
out of the touchpad face**, right-handed. Compare the raw signs with the default
axis map in `gearvr-fusion`, and record the observed map. Do not apply a guess
without the measurement.

- [ ] gravity axis identified
- [ ] X/Y signs resolved by four 90° rotations
- [ ] gyro signs agree with the right-hand rule
- [ ] observed map recorded in `implementation.md`

## 7. Sleep and reconnect (task 8.4)

Leave the stream idle and time how long the controller stays connected, then
press a button and see whether it reconnects on its own. Repeat at least ten
times, including one wake with a button held during the disconnect.

Known behaviour to confirm: sleep is triggered by lack of button/touch activity,
movement alone may not count, and keep-alives do not prevent it.

- [ ] idle time to sleep measured
- [ ] wake re-runs `0800` → `0100`
- [ ] virtual evdev nodes keep the same identity
- [ ] held keys, touchpad, and motion reset on disconnect
- [ ] ten cycles without a manual `remove`

## 8. Steam and service (tasks 8.5–8.7)

On the Deck, copy the release binary and the udev rule, then:

```sh
~/gearvr-probe/gearvr-cli run 'Gear VR' --backend evdev --motion
```

Check that Steam Input enumerates `1209/0001`, that the generic mapping is
usable, and that gesture defaults (off) do not interfere. Then install the
systemd user unit from `packaging/README.md` and verify start at login, restart
after a failed session, journal output, and a clean stop.

- [ ] Steam sees the gamepad with no stuck inputs
- [ ] gestures either left off or tuned
- [ ] service starts at login
- [ ] service restarts after a disconnect
- [ ] `systemctl --user stop` leaves no virtual devices

## 9. Record the result

Update `docs/implementation.md` and tick tasks 8.x only where an observed result
exists. Include host, kernel, BlueZ version, firmware string, controller MAC,
and the date. Keep captures under a stable name such as
`captures/YYYY-MM-DD-<mac>-first.jsonl`.
