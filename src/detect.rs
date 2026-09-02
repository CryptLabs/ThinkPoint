//! "Which device actually sent that click?"
//!
//! The physical buttons under a ThinkPad TrackPoint are not always reported by
//! the TrackPoint device — on plenty of machines they belong to the touchpad.
//! Remapping the wrong device is silent and looks like the remap simply did not
//! work, so this watches `xinput test-xi2 --root` and names the real source.

use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;
use std::time::{Duration, Instant};

use crate::error::{Error, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Hit {
    pub device_id: u32,
    pub button: u32,
}

/// A line-at-a-time state machine over the event blocks xinput prints:
///
/// ```text
/// EVENT type 4 (ButtonPress)
///     device: 13 (13)
///     detail: 2
/// ```
#[derive(Default)]
pub struct HitParser {
    in_press: bool,
    device: Option<u32>,
}

impl HitParser {
    pub fn feed(&mut self, line: &str) -> Option<Hit> {
        let trimmed = line.trim();

        if trimmed.starts_with("EVENT type") {
            // A new block begins, so anything half-parsed is stale.
            self.in_press = trimmed.contains("(ButtonPress)");
            self.device = None;
            return None;
        }
        if !self.in_press {
            return None;
        }

        if let Some(rest) = trimmed.strip_prefix("device:") {
            // "device: 13 (13)" — the first number is the source device.
            self.device = rest
                .split_whitespace()
                .next()
                .and_then(|t| t.parse::<u32>().ok());
            return None;
        }

        if let Some(rest) = trimmed.strip_prefix("detail:") {
            self.in_press = false;
            let button = rest.trim().parse::<u32>().ok()?;
            let device_id = self.device.take()?;
            return Some(Hit { device_id, button });
        }

        None
    }
}

pub struct Detector {
    child: Child,
    rx: Receiver<Hit>,
    pub hits: Vec<Hit>,
    pub ended: Option<String>,
}

/// Start `xinput test-xi2 --root` and hand back the child plus its stdout.
/// Shared by the button detector and the drift meter, which read the same
/// stream for different event types.
fn spawn_xi2() -> Result<(Child, std::process::ChildStdout)> {
    let mut child = Command::new("xinput")
        .args(["test-xi2", "--root"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                Error::new("xinput not found")
            } else {
                Error::new(format!("xinput test-xi2: {e}"))
            }
        })?;

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| Error::new("could not capture xinput output"))?;
    Ok((child, stdout))
}

impl Detector {
    pub fn start() -> Result<Self> {
        let (child, stdout) = spawn_xi2()?;
        let (tx, rx) = mpsc::channel();

        thread::spawn(move || {
            let reader = BufReader::new(stdout);
            let mut parser = HitParser::default();
            for line in reader.lines().map_while(std::result::Result::ok) {
                if let Some(hit) = parser.feed(&line)
                    && tx.send(hit).is_err()
                {
                    return;
                }
            }
        });

        Ok(Detector {
            child,
            rx,
            hits: Vec::new(),
            ended: None,
        })
    }

    /// Drain whatever the reader thread has queued. Cheap enough to call on
    /// every render tick.
    pub fn poll(&mut self) {
        loop {
            match self.rx.try_recv() {
                Ok(hit) => {
                    self.hits.push(hit);
                    if self.hits.len() > 12 {
                        self.hits.remove(0);
                    }
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    if self.ended.is_none() {
                        self.ended = Some("xinput stopped reporting events".to_string());
                    }
                    break;
                }
            }
        }
    }
}

impl Drop for Detector {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

// ---- Drift meter --------------------------------------------------------

/// Movement a device reported during one event block.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Motion {
    pub device_id: u32,
    pub dx: f64,
    pub dy: f64,
}

/// Anything larger than this in a single block is a valuator wrapping or the
/// server resetting its accumulator, not a hand on the stick.
const IMPLAUSIBLE_DELTA: f64 = 10_000.0;

/// Motion blocks carry running accumulators rather than deltas:
///
/// ```text
/// EVENT type 6 (Motion)
///     device: 13 (13)
///     valuators:
///          0: 1234.00
///          1: 5678.00
/// ```
///
/// so the delta is the difference from the last value seen on that axis. That
/// works across xinput versions, including those that append a second
/// parenthesised figure.
#[derive(Default)]
pub struct MotionParser {
    in_motion: bool,
    in_valuators: bool,
    device: Option<u32>,
    pending: Option<Motion>,
    last: HashMap<(u32, u32), f64>,
}

impl MotionParser {
    pub fn feed(&mut self, line: &str) -> Option<Motion> {
        let trimmed = line.trim();

        if trimmed.starts_with("EVENT type") {
            // A block boundary flushes whatever the previous block accumulated.
            let flushed = self.pending.take().filter(|m| m.dx != 0.0 || m.dy != 0.0);
            self.in_motion = trimmed.contains("(Motion)");
            self.in_valuators = false;
            self.device = None;
            return flushed;
        }
        if !self.in_motion {
            return None;
        }

        if let Some(rest) = trimmed.strip_prefix("device:") {
            self.device = rest
                .split_whitespace()
                .next()
                .and_then(|t| t.parse::<u32>().ok());
            return None;
        }

        if trimmed.starts_with("valuators:") {
            self.in_valuators = true;
            return None;
        }

        if !self.in_valuators {
            return None;
        }

        // Valuator rows are "N: value" and nothing else starts with a digit.
        let Some((axis, rest)) = trimmed.split_once(':') else {
            self.in_valuators = false;
            return None;
        };
        let Ok(axis) = axis.trim().parse::<u32>() else {
            self.in_valuators = false;
            return None;
        };
        let value = rest
            .split_whitespace()
            .next()
            .and_then(|t| t.parse::<f64>().ok())?;
        let device_id = self.device?;

        let delta = match self.last.insert((device_id, axis), value) {
            Some(previous) => value - previous,
            // First sighting of this axis establishes the baseline only.
            None => return None,
        };
        if delta.abs() > IMPLAUSIBLE_DELTA {
            return None;
        }

        let motion = self.pending.get_or_insert(Motion {
            device_id,
            dx: 0.0,
            dy: 0.0,
        });
        match axis {
            0 => motion.dx += delta,
            1 => motion.dy += delta,
            _ => {}
        }
        None
    }
}

/// One device's drift over the meter's rolling window.
#[derive(Debug, Clone, Copy)]
pub struct Reading {
    pub device_id: u32,
    pub dx_per_sec: f64,
    pub dy_per_sec: f64,
    pub events: usize,
}

impl Reading {
    pub fn magnitude(&self) -> f64 {
        self.dx_per_sec.hypot(self.dy_per_sec)
    }

    /// Plain words for the number, so the panel answers the question the user
    /// actually has rather than making them calibrate their own intuition.
    pub fn verdict(&self) -> &'static str {
        match self.magnitude() {
            m if m < 0.5 => "steady",
            m if m < 5.0 => "slight creep",
            m if m < 25.0 => "drifting",
            _ => "drifting badly",
        }
    }
}

pub struct Meter {
    child: Child,
    rx: Receiver<Motion>,
    samples: Vec<(Instant, Motion)>,
    pub started: Instant,
    pub ended: Option<String>,
}

/// Long enough to average out a twitch, short enough to respond while watching.
pub const METER_WINDOW: Duration = Duration::from_secs(5);

impl Meter {
    pub fn start() -> Result<Self> {
        let (child, stdout) = spawn_xi2()?;
        let (tx, rx) = mpsc::channel();

        thread::spawn(move || {
            let reader = BufReader::new(stdout);
            let mut parser = MotionParser::default();
            for line in reader.lines().map_while(std::result::Result::ok) {
                if let Some(motion) = parser.feed(&line)
                    && tx.send(motion).is_err()
                {
                    return;
                }
            }
        });

        Ok(Meter {
            child,
            rx,
            samples: Vec::new(),
            started: Instant::now(),
            ended: None,
        })
    }

    pub fn poll(&mut self) {
        let now = Instant::now();
        loop {
            match self.rx.try_recv() {
                Ok(motion) => self.samples.push((now, motion)),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    if self.ended.is_none() {
                        self.ended = Some("xinput stopped reporting events".to_string());
                    }
                    break;
                }
            }
        }
        self.samples
            .retain(|(at, _)| now.duration_since(*at) <= METER_WINDOW);
    }

    /// Per-device rates over the window, busiest first.
    pub fn readings(&self) -> Vec<Reading> {
        let elapsed = self.started.elapsed().min(METER_WINDOW).as_secs_f64();
        let seconds = if elapsed < 0.25 { 0.25 } else { elapsed };

        let mut totals: HashMap<u32, (f64, f64, usize)> = HashMap::new();
        for (_, motion) in &self.samples {
            let entry = totals.entry(motion.device_id).or_insert((0.0, 0.0, 0));
            entry.0 += motion.dx;
            entry.1 += motion.dy;
            entry.2 += 1;
        }

        let mut readings: Vec<Reading> = totals
            .into_iter()
            .map(|(device_id, (dx, dy, events))| Reading {
                device_id,
                dx_per_sec: dx / seconds,
                dy_per_sec: dy / seconds,
                events,
            })
            .collect();
        readings.sort_by(|a, b| {
            b.magnitude()
                .partial_cmp(&a.magnitude())
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        readings
    }
}

impl Drop for Meter {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hits(sample: &str) -> Vec<Hit> {
        let mut parser = HitParser::default();
        sample.lines().filter_map(|l| parser.feed(l)).collect()
    }

    #[test]
    fn reads_the_source_device_of_a_press() {
        let sample = "\
EVENT type 15 (RawButtonPress)
    device: 2 (13)
    detail: 2
EVENT type 4 (ButtonPress)
    device: 13 (13)
    detail: 2
    root: 1920/1080
";
        assert_eq!(
            hits(sample),
            vec![Hit {
                device_id: 13,
                button: 2
            }]
        );
    }

    #[test]
    fn ignores_motion_and_release_blocks() {
        let sample = "\
EVENT type 6 (Motion)
    device: 13 (13)
    detail: 0
EVENT type 5 (ButtonRelease)
    device: 13 (13)
    detail: 2
";
        assert!(hits(sample).is_empty());
    }

    #[test]
    fn a_truncated_block_does_not_leak_into_the_next() {
        let sample = "\
EVENT type 4 (ButtonPress)
    device: 12 (12)
EVENT type 4 (ButtonPress)
    device: 13 (13)
    detail: 3
";
        assert_eq!(
            hits(sample),
            vec![Hit {
                device_id: 13,
                button: 3
            }]
        );
    }

    fn motions(sample: &str) -> Vec<Motion> {
        let mut parser = MotionParser::default();
        sample.lines().filter_map(|l| parser.feed(l)).collect()
    }

    // Valuators are running accumulators, so the first block only establishes
    // a baseline and the second is the first real measurement.
    const DRIFT: &str = "\
EVENT type 6 (Motion)
    device: 13 (13)
    detail: 0
    valuators:
         0: 1000.00
         1: 2000.00
EVENT type 6 (Motion)
    device: 13 (13)
    detail: 0
    valuators:
         0: 1003.00
         1: 1998.00
EVENT type 6 (Motion)
    device: 13 (13)
    detail: 0
    valuators:
         0: 1005.00
         1: 1998.00
";

    #[test]
    fn turns_valuator_accumulators_into_deltas() {
        let seen = motions(DRIFT);
        assert_eq!(
            seen.len(),
            1,
            "one flushed block, the last is still pending"
        );
        assert_eq!(seen[0].device_id, 13);
        assert_eq!(seen[0].dx, 3.0);
        assert_eq!(seen[0].dy, -2.0);
    }

    #[test]
    fn the_first_block_is_only_a_baseline() {
        let first_only = "\
EVENT type 6 (Motion)
    device: 13 (13)
    valuators:
         0: 1000.00
EVENT type 6 (Motion)
";
        assert!(motions(first_only).is_empty());
    }

    #[test]
    fn a_valuator_reset_is_not_reported_as_a_lurch() {
        let sample = "\
EVENT type 6 (Motion)
    device: 13 (13)
    valuators:
         0: 1000.00
EVENT type 6 (Motion)
    device: 13 (13)
    valuators:
         0: 900000.00
EVENT type 6 (Motion)
";
        assert!(
            motions(sample).is_empty(),
            "an implausible jump is discarded"
        );
    }

    #[test]
    fn button_blocks_do_not_register_as_motion() {
        let sample = "\
EVENT type 4 (ButtonPress)
    device: 13 (13)
    detail: 2
    valuators:
         0: 1000.00
EVENT type 4 (ButtonPress)
    device: 13 (13)
    detail: 2
    valuators:
         0: 1050.00
EVENT type 6 (Motion)
";
        assert!(motions(sample).is_empty());
    }

    #[test]
    fn two_devices_are_measured_separately() {
        let sample = "\
EVENT type 6 (Motion)
    device: 12 (12)
    valuators:
         0: 100.00
EVENT type 6 (Motion)
    device: 13 (13)
    valuators:
         0: 500.00
EVENT type 6 (Motion)
    device: 12 (12)
    valuators:
         0: 110.00
EVENT type 6 (Motion)
    device: 13 (13)
    valuators:
         0: 501.00
EVENT type 6 (Motion)
";
        let seen = motions(sample);
        assert_eq!(seen.len(), 2);
        assert_eq!((seen[0].device_id, seen[0].dx), (12, 10.0));
        assert_eq!((seen[1].device_id, seen[1].dx), (13, 1.0));
    }

    #[test]
    fn verdicts_track_magnitude() {
        let reading = |dx: f64| Reading {
            device_id: 13,
            dx_per_sec: dx,
            dy_per_sec: 0.0,
            events: 1,
        };
        assert_eq!(reading(0.0).verdict(), "steady");
        assert_eq!(reading(2.0).verdict(), "slight creep");
        assert_eq!(reading(10.0).verdict(), "drifting");
        assert_eq!(reading(100.0).verdict(), "drifting badly");
        assert_eq!(reading(-100.0).verdict(), "drifting badly");
    }

    #[test]
    fn distinguishes_the_touchpad_from_the_trackpoint() {
        // The case this exists for: a middle click arriving from the touchpad
        // even though the TrackPoint is the device being remapped.
        let sample = "\
EVENT type 4 (ButtonPress)
    device: 12 (12)
    detail: 2
";
        assert_eq!(hits(sample)[0].device_id, 12);
    }
}
