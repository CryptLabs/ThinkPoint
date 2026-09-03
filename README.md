# ThinkPoint

A terminal interface for the pointer settings that normally live in scattered
sysfs files and half-remembered `xinput` incantations — TrackPoint sensitivity,
button maps, libinput properties — with a way to make each of them stick.

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

From the AUR:

```sh
yay -S thinkpoint
```

From source:

```sh
cargo build --release
install -Dm755 target/release/thinkpoint /usr/local/bin/thinkpoint
```

Runtime dependencies: `xinput` for anything X11-side and `sudo` for writing
sysfs and udev files. Without `xinput` the tool falls back to sysfs-only
tuning.

## Keys

| Key | Does |
| --- | --- |
| `↑ ↓` / `j k` | move within the focused pane |
| `← →` | switch pane, or adjust the selected value |
| `tab` / `shift-tab` | cycle Buttons · libinput · sysfs |
| `space` | toggle: disable a button, flip an on/off setting |
| `e` | type a value |
| `t` | turn the selected device off or on |
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

## On drift

Pressing `p` stages the two things the kernel can do about TrackPoint drift:

- `sensitivity` drops to three quarters of its current value, floored at 40.
  This does not stop the underlying creep. It scales down the motion that the
  same spurious force produces, which is usually enough to stop it being
  noticeable, and below 40 the stick gets unusable well before the drift stops
  mattering.
- `drift_time` rises to 20 — but only where the device has one.

That caveat is the whole story on most recent ThinkPads. `drift_time` is the
actual drift-correction window, and the kernel's trackpoint driver only exposes
it for genuine IBM TrackPoints; Elan, ALPS and NXP variants get `sensitivity`
and `press_to_select` and nothing else. On those, drift correction happens in
firmware where nothing in userspace can reach it. The status bar says so rather
than implying the preset did more than it did, and what remains is the physical
side: reseat or replace the cap, and check for a BIOS update, since Lenovo has
shipped TrackPoint calibration fixes that way.

Use `m` before and after to see whether any of it helped.

## Making it stick

The two kinds of setting persist differently, because they live in different
places.

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

**Button maps, libinput properties and the on/off state belong to the X
server**, and udev cannot reach them. Those go to
`~/.config/thinkpoint/profile.conf`, replayed with:

```sh
thinkpoint --restore
```

Wire that into whatever starts your session — for i3:

```
exec_always --no-startup-id thinkpoint --restore
```

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

Wayland: the sysfs tab works, everything X11-side does not. There is no
equivalent of a per-device button map to configure from outside the compositor.

## Licence

MIT.
