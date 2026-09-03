# Changelog

Notable changes to ThinkPoint. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project uses
[semantic versioning](https://semver.org/spec/v2.0.0.html) — while the major
version is 0, a minor bump may still change behaviour.

## [0.2.0] — 2026-09-03

### Added

- Middle-button chooser (`b`): turn pasting and scrolling on or off
  independently. They look like one setting and are two, in different places —
  pasting is the X button map delivering button 2 to applications, scrolling is
  libinput taking that button before the map is consulted.
- Device on/off toggle (`t`), for silencing a touchpad without touching any of
  its other settings. Applies immediately and is remembered in the profile, so
  `--restore` reapplies it.
- Drift meter (`m`): per-device pointer creep in counts per second per axis,
  read from the device's own valuators so the figure is pre-acceleration and
  describes the hardware rather than the cursor.
- Drift-reducing preset (`p`): stages `sensitivity` at three quarters of its
  current value, floored at 40, and raises `drift_time` to 20 where the device
  has one. It says plainly when the device has no `drift_time`, which is the
  case on Elan, ALPS and NXP TrackPoints.
- A password prompt inside the interface for writes that need root.
- GitHub Actions workflow running fmt, clippy, tests and a release build.

### Changed

- Root writes go through `sudo` rather than `pkexec`: a direct write first,
  then `sudo -n` for NOPASSWD or a live timestamp, then the prompt. This drops
  the dependency on a polkit agent, which under a bare window manager is often
  not running. `sudo` is now a hard dependency; `polkit` is no longer an
  optional one.
- The password reaches sudo on standard input, never a command line or a file,
  and is overwritten in memory once sudo has taken it.

### Fixed

- A disabled device could not be re-enabled after restarting the tool. The
  toggle trusted the enabled state cached at start-up, so a stale value
  inverted the action; it now reads the state immediately before acting and
  verifies the result afterwards.
- A disabled device could appear twice — once as the real X device and once as
  a dead sysfs-only row with no way to switch it back on — because a disabled
  device may stop reporting a `Device Node` property, leaving its serio node
  looking unclaimed. Serio nodes now also match on the input device names
  underneath them.

## [0.1.0] — 2026-09-02

First release.

### Added

- Per-device button maps, with a staged-then-applied editing model and a
  read-back check that catches the X server accepting a call but keeping the
  old map.
- libinput property editing: acceleration, natural scrolling, middle-button
  emulation and whatever else a device exposes.
- Kernel-side TrackPoint tuning through sysfs, discovering which attributes are
  actually present rather than assuming a fixed set.
- Button source detector (`d`), naming the device that really sent a click.
- Persistence: a udev rule for sysfs attributes, and a profile replayed by
  `--restore` for the X-side settings udev cannot reach.
- `--print-rule` and `--print-profile`.

[0.2.0]: https://github.com/CryptLabs/ThinkPoint/releases/tag/v0.2.0
[0.1.0]: https://github.com/CryptLabs/ThinkPoint/releases/tag/v0.1.0
