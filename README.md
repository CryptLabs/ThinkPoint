<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="assets/wordmark-dark.png">
    <img alt="ThinkPoint" src="assets/wordmark-light.png" width="420">
  </picture>
</p>

A terminal interface for the pointer settings that normally live in scattered
sysfs files and half-remembered `xinput` incantations — TrackPoint sensitivity,
button maps, libinput properties — with a way to make each of them stick.

It came out of two ThinkPad annoyances: a TrackPoint that drifts, and a middle
button that pastes the selection when all you wanted was to scroll. There is a
[section on drift](#fixing-trackpoint-drift) that walks through measuring it,
what the kernel can actually change, and where the fix stops being software.

Written for ThinkPads, but it works on any Linux machine with pointer devices.

```
 ThinkPoint   buttons, libinput and TrackPoint tuning
╭ Devices ─────────────────────────────╮╭ TPPS/2 Elan TrackPoint ─────────────────────────────╮
│TPPS/2 Elan TrackPoint ●              ││ Buttons │ libinput │ sysfs                          │
│  id 13 · 7 buttons · sysfs           ││sensitivity       90  [0–255]  live: 128  root       │
│SynPS/2 Synaptics TouchPad            ││press_to_select    0  [0/1]  root                    │
│  id 12 · 7 buttons · sysfs           ││                                                     │
│                                      ││─────────────────────────────────────────────────────│
│                                      ││Force needed to move the pointer.                    │
│                                      ││ equivalent command ─────────────────────────────────│
│                                      ││echo 90 | sudo tee /sys/.../serio1/sensitivity       │
╰──────────────────────────────────────╯╰─────────────────────────────────────────────────────╯
```

## What it does

**Button maps.** Disable a button per device, most usefully button 2 to stop
middle-click pasting the primary selection. TrackPoint scrolling survives that:
libinput consumes the button for scrolling before the X button map is applied,
and scroll travels as buttons 4–7.

**A middle-button chooser.** Press `b` to turn pasting and scrolling on or off
independently for the middle button. They look like one setting and are two:
pasting is the X button map delivering button 2 to applications, scrolling is
libinput taking that button before the map is consulted. Scroll on with paste
off — the stick still scrolls, a middle click no longer dumps the selection
into whatever has focus — is the combination most people are after.

**Turning a device off.** Press `t` to disable the selected device, `t` again
to bring it back — the usual reason being a touchpad you keep brushing while
typing. It takes effect at once, leaves every other setting on the device
untouched, and is remembered in the profile so `--restore` can switch it off
again next session.

**libinput properties.** Acceleration, natural scrolling, middle-button
emulation and whatever else the driver exposes on a given device, edited in
place rather than looked up in `xinput list-props` output.

**Kernel-side TrackPoint tuning.** `sensitivity`, `press_to_select` and, on IBM
sticks, the full set — `drift_time`, `inertia`, `thresh` and the rest. The tool
discovers what is actually present, because the kernel's trackpoint driver only
exposes the full set for genuine IBM variants; Elan, ALPS and NXP sticks get
just two attributes, which is why `drift_time` is missing on most recent
ThinkPads.

**A source detector.** Press `d` and click a button: the tool names the device
that really sent it. The physical buttons under a TrackPoint often belong to the
touchpad, and remapping the wrong device fails silently in a way that looks like
the remap simply did not work.

**A drift meter.** Press `m`, take your hands off the machine, and read the
pointer creep in counts per second per axis, straight from the device's own
valuators — before pointer acceleration, so the figure describes the hardware
rather than what the cursor did. A still reading with a visibly creeping cursor
means the cause is above the driver.

Every screen shows the equivalent shell command for whatever is selected, so the
tool teaches rather than hides.

## Install

Nothing here is Arch-specific. It is one Rust binary that shells out to
`xinput` and `sudo`.

**Prebuilt binary** — from the [releases
page](https://github.com/CryptLabs/ThinkPoint/releases), for any distribution:

```sh
tar xzf thinkpoint-*-x86_64-unknown-linux-gnu.tar.gz
install -Dm755 thinkpoint-*/thinkpoint ~/.local/bin/thinkpoint
```

x86_64 and aarch64 builds are published on each tag, with `SHA256SUMS`
alongside them.

**Cargo**, on any distribution with a Rust toolchain:

```sh
cargo install thinkpoint
```

**Arch and derivatives:**

```sh
yay -S thinkpoint
```

**Debian and Ubuntu** — a `.deb` is attached to each release:

```sh
sudo apt install ./thinkpoint_*_amd64.deb
```

**From source:**

```sh
cargo build --release
install -Dm755 target/release/thinkpoint /usr/local/bin/thinkpoint
```

Runtime dependencies: `xinput` for anything X11-side and `sudo` for writing
sysfs and udev files. Without `xinput` the tool falls back to sysfs-only
tuning.

## Platform

Linux only, and X11 for most of it.

The device-side features — button maps, libinput properties, the on/off
toggle, the detector and the drift meter — all go through the X server. Under
Wayland the sysfs tab still works, because that talks to the kernel, but
nothing else does: there is no equivalent of a per-device button map to
configure from outside the compositor.

The sysfs tuning is Linux-specific by nature, so there is no BSD or macOS
build.

## Keys

| Key | Does |
| --- | --- |
| `↑ ↓` / `j k` | move within the focused pane |
| `← →` | switch pane, or adjust the selected value |
| `tab` / `shift-tab` | cycle Buttons · libinput · sysfs |
| `space` | toggle: disable a button, flip an on/off setting |
| `e` | type a value |
| `t` | turn the selected device off or on |
| `i` | about: version, links and what this session found |
| `b` | middle button: paste and scroll, chosen separately |
| `p` | stage the drift-reducing preset on this device |
| `m` | measure drift with your hands off the machine |
| `a` | apply everything staged in this section |
| `u` | reset the button map to how it was at start-up |
| `s` | save — udev rule for sysfs, profile for X settings |
| `d` | detect which device sends a button press |
| `r` | re-read everything from the system |
| `q` | quit |

Changes are staged and applied with `a`, so nothing moves under you while you
are looking at it. Nothing is written to disk except from the `s` screen.

## Root access

Writing sysfs attributes and the udev rule needs root. ThinkPoint tries, in
order: a direct write, in case you are already root; `sudo -n`, which succeeds
if you have NOPASSWD or a still-valid sudo timestamp; and failing both, it asks
for your password in a prompt inside the interface.

The password goes to `sudo` on standard input. It never reaches a command line
where `ps` would show it, never touches a file, and is overwritten in memory as
soon as sudo has taken it. Because sudo caches its own authentication for a few
minutes, one prompt usually covers a whole session of tinkering.

There is no dependency on a polkit agent, which under a bare window manager is
often not running at all.

## Fixing TrackPoint drift

Drift is the pointer creeping on its own with nothing touching the stick. It is
the problem this program was written for, so it is worth being clear about what
it can and cannot do.

**First, find out whether it is really the hardware.** Press `m`, take your
hands off the machine, and read the number. The meter reports movement per
second per axis straight from the device's own valuators, before pointer
acceleration, so it describes the stick rather than the cursor.

- A reading of **zero while the cursor still creeps** means the drift is coming
  from above the driver. The device is fine; look at the accel profile on the
  libinput tab.
- A **steady non-zero reading** is genuine device drift, and the rest of this
  applies.

**Then try the preset.** `p` stages the two things the kernel can do, `a`
applies them:

- `sensitivity` drops to three quarters of its current value, floored at 40.
  This does not stop the creep. It scales down the motion that the same
  spurious force produces, which is often enough to stop it being noticeable.
  Below 40 the stick gets unusable well before the drift stops mattering, which
  is why the floor is there.
- `drift_time` rises to 20 — the actual drift-correction window — **but only on
  devices that have one.**

Press `m` again. If the number has dropped and the stick still feels usable,
you are done; save it with `s` so it survives a reboot. If sensitivity made no
difference to the measured drift, the creep is not proportional to it and no
amount of further lowering will help.

**Where it runs out.** `drift_time` is exposed by the kernel's trackpoint
driver only for genuine IBM TrackPoints. Elan, ALPS and NXP variants — which is
most ThinkPads made in the last several years — get `sensitivity` and
`press_to_select` and nothing else. On those, drift correction happens in
firmware, where nothing in userspace can reach it. ThinkPoint says so in the
status bar rather than implying the preset did more than it did, and the sysfs
tab simply will not show a `drift_time` row.

If you have reached that point, the remaining causes are physical and this
program cannot help with any of them:

- **A worn or badly seated cap.** A loose or split cap, or grit under it, puts
  a constant off-centre load on the sensor. Pull it off, clean the post, press a
  fresh one on. Caps are cheap and this is the single most common cause.
- **Recalibration.** The stick re-zeroes itself when it detects no force. Rest a
  finger on it while that is happening and it latches a bad zero. Lift off
  completely for a few seconds. Drift just after boot, or as the machine warms
  up, is the same effect — the strain gauges are temperature-sensitive and it
  settles.
- **Firmware.** Lenovo has shipped TrackPoint calibration fixes in BIOS updates
  more than once. Worth checking before concluding the hardware is faulty.
- **A failing sensor.** If it drifts in the BIOS setup screen too, it is not a
  software problem at all, and the fix is a keyboard assembly replacement.

## Making it stick

The two kinds of setting persist differently, because they live in different
places.

Applying with `a` changes the running system only. Nothing on the sysfs tab
survives a reboot until you save it with `s` — that is the step that writes the
udev rule.

**sysfs attributes belong to the kernel**, so `s` generates a udev rule at
`/etc/udev/rules.d/70-thinkpoint.rules`:

```
ACTION=="add", SUBSYSTEM=="serio", DRIVERS=="psmouse", ATTR{sensitivity}="90"
```

This survives reboot and, unlike a session hook, device re-enumeration after
suspend. Reload without rebooting:

```sh
sudo udevadm control --reload
sudo udevadm trigger --subsystem-match=serio
```

The generated file includes a commented, narrower variant matched on
`firmware_id`, for machines where more than one psmouse device is on the bus and
the broad match is not what you want.

Saving reads the existing rule first and carries across anything it is not
changing. That matters after a reboot: an attribute persisted last session
reads back as the boot value, so it looks unchanged, and a naive regeneration
would drop it while saving something else.

Hand edits to the rule file survive too, as long as they are `ATTR{...}="..."`
assignments on the live line.

**Button maps, libinput properties and the on/off state belong to the X
server**, and udev cannot reach them — including both halves of the
middle-button choice, since pasting is the button map and scrolling is a
libinput property. Those go to `~/.config/thinkpoint/profile.conf`, replayed
with:

```sh
thinkpoint --restore
```

Wire that into whatever starts your session — for i3:

```
exec_always --no-startup-id thinkpoint --restore
```

Without that line nothing on the X side comes back after a login, however many
times you save. Saving merges with the existing profile rather than replacing
it, for the same reason the udev rule does.

## Command line

```
thinkpoint                  open the terminal interface
thinkpoint --restore        reapply saved button maps and libinput properties
thinkpoint --print-rule     print a udev rule for the current sysfs values
thinkpoint --print-profile  print the saved X profile
```

## Notes

`--print-rule` emits every tunable attribute it can see with its current value,
so you can trim it by hand; the rule written from inside the tool contains only
what you changed.

A disabled device keeps its settings and stays in the list, struck through and
marked `off`; it just stops reaching applications. Nothing about it is
destructive, and `t` undoes it.

A device can also end up *detached* — floating, in X's terms, meaning attached
to no master pointer and therefore driving no cursor. ThinkPoint shows those
marked `detached`, and `t` reattaches and enables them in one go. This is worth
knowing because `xinput list` labels a floating device `[floating slave]` with
no mention of "pointer", so tools that look for slave pointers do not see it at
all and it appears to have vanished.

Wayland: the sysfs tab works, everything X11-side does not. There is no
equivalent of a per-device button map to configure from outside the compositor.

## Changelog

See [CHANGELOG.md](CHANGELOG.md).

## About screen

`i` shows the version, the author, the links and a short summary of what this
session found: whether `xinput` is available, how many devices were listed, how
many have kernel-side tuning, and whether sudo will ask for a password. Those
are the first things anyone needs in a bug report and the last things anyone
thinks to look up, so they are one key away.

The version also sits in the title bar, and `--version` prints it.

## Licence

MIT.
