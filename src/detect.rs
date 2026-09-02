//! "Which device actually sent that click?"
//!
//! The physical buttons under a ThinkPad TrackPoint are not always reported by
//! the TrackPoint device — on plenty of machines they belong to the touchpad.
//! Remapping the wrong device is silent and looks like the remap simply did not
//! work, so this watches `xinput test-xi2 --root` and names the real source.

use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;

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

impl Detector {
    pub fn start() -> Result<Self> {
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
