//! Reading and writing the psmouse/trackpoint knobs under /sys.
//!
//! Which attributes exist depends on the TrackPoint variant. The kernel's
//! trackpoint driver only exposes the full set (drift_time, inertia, thresh,
//! and friends) for genuine IBM sticks; Elan, ALPS and NXP variants get just
//! `sensitivity` and `press_to_select`. That is why this module discovers what
//! is present rather than assuming a fixed list.

use std::fs;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::error::{Error, Result, ensure_shell_safe};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttrKind {
    Range(i64, i64),
    Bool,
}

#[derive(Debug, Clone)]
pub struct Attr {
    pub name: String,
    pub help: &'static str,
    pub kind: AttrKind,
    /// Value as it currently reads from the kernel.
    pub value: String,
    /// Value as it read when the tool started — what "changed" is measured
    /// against, and what decides whether an attribute is worth persisting.
    pub original: String,
    /// Staged value, applied when the user presses `a`.
    pub pending: String,
    /// Whether this process can write it without elevation.
    pub writable_directly: bool,
    pub path: PathBuf,
}

impl Attr {
    /// Staged but not yet written to the kernel.
    pub fn is_dirty(&self) -> bool {
        self.pending != self.value
    }

    /// Differs from how the machine booted, so worth writing into a udev rule.
    pub fn is_customised(&self) -> bool {
        self.pending != self.original
    }

    pub fn as_int(&self) -> Option<i64> {
        self.pending.trim().parse().ok()
    }

    pub fn nudge(&mut self, delta: i64) {
        let (lo, hi) = match self.kind {
            AttrKind::Range(lo, hi) => (lo, hi),
            AttrKind::Bool => (0, 1),
        };
        let current = self.as_int().unwrap_or(lo);
        let next = (current + delta).clamp(lo, hi);
        self.pending = next.to_string();
    }

    pub fn toggle(&mut self) {
        if self.kind == AttrKind::Bool {
            let current = self.as_int().unwrap_or(0);
            self.pending = if current == 0 { "1" } else { "0" }.to_string();
        }
    }
}

/// Attributes we know how to describe, in the order we want to show them.
/// Anything not listed here is ignored, which keeps `uevent`, `modalias` and
/// the other plumbing files out of the UI.
const KNOWN: &[(&str, &str, AttrKind)] = &[
    (
        "sensitivity",
        "Force needed to move the pointer. Lower values reduce visible drift.",
        AttrKind::Range(0, 255),
    ),
    (
        "speed",
        "Pointer speed multiplier applied inside the device.",
        AttrKind::Range(0, 255),
    ),
    (
        "press_to_select",
        "Tapping the stick acts as a click.",
        AttrKind::Bool,
    ),
    (
        "drift_time",
        "Drift-correction window in ~20ms units. IBM TrackPoints only.",
        AttrKind::Range(0, 255),
    ),
    (
        "inertia",
        "Negative inertia factor: how sharply motion decays.",
        AttrKind::Range(0, 255),
    ),
    (
        "reach",
        "Backup range for the press-to-select gesture.",
        AttrKind::Range(0, 255),
    ),
    (
        "draghys",
        "Drag hysteresis: resistance before a drag starts.",
        AttrKind::Range(0, 255),
    ),
    (
        "mindrag",
        "Minimum force that will sustain a drag.",
        AttrKind::Range(0, 255),
    ),
    (
        "thresh",
        "Movement threshold before the stick reports anything.",
        AttrKind::Range(0, 255),
    ),
    (
        "upthresh",
        "Force threshold for press-to-select.",
        AttrKind::Range(0, 255),
    ),
    (
        "ztime",
        "Timing window for press-to-select.",
        AttrKind::Range(0, 255),
    ),
    (
        "jenks",
        "Jenks curvature: shape of the force-to-speed curve.",
        AttrKind::Range(0, 255),
    ),
    (
        "skipback",
        "Suppress the backup movement after a press-to-select.",
        AttrKind::Bool,
    ),
    (
        "ext_dev",
        "External pointing device passthrough.",
        AttrKind::Bool,
    ),
    ("rate", "PS/2 report rate in Hz.", AttrKind::Range(0, 200)),
    (
        "resolution",
        "PS/2 resolution in counts per millimetre.",
        AttrKind::Range(0, 8),
    ),
    (
        "resetafter",
        "Reset the device after this many bad packets. 0 disables.",
        AttrKind::Range(0, 255),
    ),
    (
        "resync_time",
        "Seconds before psmouse tries to resync a wedged device.",
        AttrKind::Range(0, 255),
    ),
];

#[derive(Debug, Clone)]
pub struct Node {
    pub path: PathBuf,
    pub description: String,
    pub firmware_id: String,
    /// Names of the input devices this serio node produces, e.g.
    /// "TPPS/2 Elan TrackPoint". Used to recognise a node as belonging to a
    /// device already listed by xinput, even when the X server is no longer
    /// reporting a device node for it.
    pub input_names: Vec<String>,
    pub attrs: Vec<Attr>,
}

impl Node {
    pub fn label(&self) -> String {
        let base = self
            .path
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.path.display().to_string());
        if self.description.is_empty() {
            base
        } else {
            format!("{base} — {}", self.description)
        }
    }

    pub fn dirty(&self) -> bool {
        self.attrs.iter().any(|a| a.is_dirty())
    }
}

fn read_trimmed(path: &Path) -> Option<String> {
    fs::read_to_string(path)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn writable_directly(path: &Path) -> bool {
    // Cheap approximation: root can write anything, otherwise look for the
    // user write bit. Getting it wrong only costs a fallback to sudo.
    if unsafe { libc_geteuid() } == 0 {
        return true;
    }
    fs::metadata(path)
        .map(|m| m.permissions().mode() & 0o200 != 0)
        .unwrap_or(false)
}

// Avoiding a libc dependency for a single call.
unsafe fn libc_geteuid() -> u32 {
    unsafe extern "C" {
        fn geteuid() -> u32;
    }
    unsafe { geteuid() }
}

/// Every `input/inputN/name` under a serio node.
fn input_names(path: &Path) -> Vec<String> {
    let mut names = Vec::new();
    let Ok(entries) = fs::read_dir(path.join("input")) else {
        return names;
    };
    for entry in entries.flatten() {
        if let Some(name) = read_trimmed(&entry.path().join("name")) {
            names.push(name);
        }
    }
    names
}

/// Build a Node for a sysfs directory, if it holds any attribute we know.
pub fn node_at(path: &Path) -> Option<Node> {
    let mut attrs = Vec::new();
    for (name, help, kind) in KNOWN {
        let file = path.join(name);
        if !file.exists() {
            continue;
        }
        let Some(value) = read_trimmed(&file) else {
            continue;
        };
        // Some attributes read back as text (protocol, for one). Skip anything
        // that is not a plain integer, since the editors here assume numbers.
        if value.parse::<i64>().is_err() {
            continue;
        }
        attrs.push(Attr {
            name: (*name).to_string(),
            help,
            kind: *kind,
            value: value.clone(),
            original: value.clone(),
            pending: value,
            writable_directly: writable_directly(&file),
            path: file,
        });
    }

    if attrs.is_empty() {
        return None;
    }

    Some(Node {
        description: read_trimmed(&path.join("description")).unwrap_or_default(),
        firmware_id: read_trimmed(&path.join("firmware_id")).unwrap_or_default(),
        input_names: input_names(path),
        path: path.to_path_buf(),
        attrs,
    })
}

/// Walk up from `/dev/input/eventN` to the serio device that owns it.
pub fn node_for_event(dev_node: &str) -> Option<Node> {
    let name = dev_node.rsplit('/').next()?;
    if !name.starts_with("event") {
        return None;
    }
    let resolved = fs::canonicalize(format!("/sys/class/input/{name}")).ok()?;
    for ancestor in resolved.ancestors() {
        if let Some(node) = node_at(ancestor) {
            return Some(node);
        }
    }
    None
}

/// Every psmouse-ish serio device on the machine, for the case where xinput is
/// unavailable and we have nothing to correlate against.
pub fn scan_all() -> Vec<Node> {
    let mut nodes = Vec::new();
    let Ok(entries) = fs::read_dir("/sys/bus/serio/devices") else {
        return nodes;
    };
    let mut paths: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
    paths.sort();
    for path in paths {
        let resolved = fs::canonicalize(&path).unwrap_or(path);
        if let Some(node) = node_at(&resolved) {
            nodes.push(node);
        }
    }
    nodes
}

/// How a write was carried out, so the UI can say something accurate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteVia {
    Direct,
    /// sudo accepted us without asking: either NOPASSWD, or a timestamp still
    /// valid from an earlier authentication.
    SudoCached,
    SudoPassword,
}

pub fn is_root() -> bool {
    unsafe { libc_geteuid() == 0 }
}

/// Can we run sudo without anyone typing anything?
pub fn sudo_ready() -> bool {
    Command::new("sudo")
        .args(["-n", "true"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Run a shell snippet as root.
///
/// The ladder is: sudo without a prompt, then sudo with the password we were
/// given on stdin. The password never appears in the argument list, where it
/// would be visible to anyone running ps.
fn run_as_root(script: &str, password: Option<&str>, stdin_data: Option<&str>) -> Result<WriteVia> {
    let cached = sudo_ready();
    if password.is_none() && !cached {
        return Err(Error::needs_password());
    }

    let mut command = Command::new("sudo");
    if cached && password.is_none() {
        command.arg("-n");
    } else {
        // -S reads the password from stdin, -p '' suppresses sudo's own prompt
        // since we have already asked for it ourselves.
        command.args(["-S", "-p", ""]);
    }
    command
        .arg("/bin/sh")
        .arg("-c")
        .arg(script)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());

    let mut child = command.spawn().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            Error::new("sudo not found")
        } else {
            Error::new(format!("sudo: {e}"))
        }
    })?;

    if let Some(stdin) = child.stdin.as_mut() {
        if let Some(password) = password {
            stdin.write_all(password.as_bytes())?;
            stdin.write_all(b"\n")?;
        }
        if let Some(data) = stdin_data {
            stdin.write_all(data.as_bytes())?;
        }
    }
    drop(child.stdin.take());

    let output = child.wait_with_output()?;
    if output.status.success() {
        return Ok(if password.is_some() {
            WriteVia::SudoPassword
        } else {
            WriteVia::SudoCached
        });
    }

    let stderr = String::from_utf8_lossy(&output.stderr).to_lowercase();
    if stderr.contains("incorrect password") || stderr.contains("sorry, try again") {
        return Err(Error::new("that password was not accepted"));
    }
    Err(Error::new("sudo refused the command"))
}

pub fn write_attr(attr: &Attr, password: Option<&str>) -> Result<WriteVia> {
    ensure_shell_safe(&attr.pending)?;

    if attr.writable_directly {
        match fs::OpenOptions::new().write(true).open(&attr.path) {
            Ok(mut f) => {
                f.write_all(attr.pending.as_bytes())?;
                return Ok(WriteVia::Direct);
            }
            Err(e) if e.kind() != std::io::ErrorKind::PermissionDenied => {
                return Err(e.into());
            }
            Err(_) => {}
        }
    }

    let path = attr.path.to_string_lossy();
    if path.contains('\'') {
        return Err(Error::new("refusing to shell out with a quote in the path"));
    }
    let script = format!("printf '%s' '{}' > '{}'", attr.pending, path);
    run_as_root(&script, password, None)
}

/// Write a file that needs root, with the content on stdin so it
/// never has to survive a trip through the shell.
pub fn write_root_file(target: &str, content: &str, password: Option<&str>) -> Result<WriteVia> {
    if unsafe { libc_geteuid() } == 0 {
        if let Some(parent) = Path::new(target).parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(target, content)?;
        return Ok(WriteVia::Direct);
    }

    if target.contains('\'') {
        return Err(Error::new("refusing to shell out with a quote in the path"));
    }
    // The content follows the password down the same pipe, so it never has to
    // survive a trip through the shell.
    let script = format!("cat > '{target}'");
    run_as_root(&script, password, Some(content))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("thinkpoint-test-{name}"));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn put(dir: &Path, name: &str, value: &str) {
        fs::write(dir.join(name), value).unwrap();
    }

    #[test]
    fn discovers_only_the_attributes_that_exist() {
        // An Elan TrackPoint: sensitivity and press_to_select, no drift_time.
        let dir = scratch("elan");
        put(&dir, "sensitivity", "128\n");
        put(&dir, "press_to_select", "0\n");
        put(&dir, "resync_time", "0\n");
        put(&dir, "description", "i8042 AUX port\n");

        let node = node_at(&dir).expect("should find a node");
        let names: Vec<&str> = node.attrs.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(names, vec!["sensitivity", "press_to_select", "resync_time"]);
        assert_eq!(node.description, "i8042 AUX port");
    }

    #[test]
    fn an_ibm_trackpoint_exposes_drift_time_too() {
        let dir = scratch("ibm");
        put(&dir, "sensitivity", "128");
        put(&dir, "drift_time", "5");
        put(&dir, "inertia", "6");

        let node = node_at(&dir).unwrap();
        assert!(node.attrs.iter().any(|a| a.name == "drift_time"));
    }

    #[test]
    fn a_directory_with_nothing_known_is_not_a_node() {
        let dir = scratch("empty");
        put(&dir, "modalias", "serio:ty01pr00id00ex00");
        put(&dir, "uevent", "MODALIAS=serio");
        assert!(node_at(&dir).is_none());
    }

    #[test]
    fn non_numeric_attributes_are_skipped() {
        let dir = scratch("textual");
        put(&dir, "sensitivity", "128");
        // `protocol` reads back as a name, not a number.
        put(&dir, "rate", "PS/2");
        let node = node_at(&dir).unwrap();
        assert_eq!(node.attrs.len(), 1);
    }

    #[test]
    fn nudging_clamps_to_the_documented_range() {
        let dir = scratch("clamp");
        put(&dir, "sensitivity", "250");
        let mut node = node_at(&dir).unwrap();
        let attr = &mut node.attrs[0];
        attr.nudge(20);
        assert_eq!(attr.pending, "255");
        attr.nudge(-1000);
        assert_eq!(attr.pending, "0");
    }

    #[test]
    fn booleans_toggle_rather_than_count() {
        let dir = scratch("bool");
        put(&dir, "press_to_select", "0");
        let mut node = node_at(&dir).unwrap();
        let attr = &mut node.attrs[0];
        attr.toggle();
        assert_eq!(attr.pending, "1");
        attr.toggle();
        assert_eq!(attr.pending, "0");
    }

    #[test]
    fn dirty_and_customised_are_different_questions() {
        let dir = scratch("dirty");
        put(&dir, "sensitivity", "128");
        let mut node = node_at(&dir).unwrap();
        let attr = &mut node.attrs[0];

        attr.pending = "90".into();
        assert!(attr.is_dirty(), "staged but not written");
        assert!(attr.is_customised());

        // Simulate a successful write: the kernel now agrees with us.
        attr.value = "90".into();
        assert!(!attr.is_dirty(), "written, so no longer staged");
        assert!(
            attr.is_customised(),
            "still differs from boot, so worth saving"
        );
    }

    #[test]
    fn a_node_reports_the_input_devices_behind_it() {
        let dir = scratch("names");
        put(&dir, "sensitivity", "128");
        fs::create_dir_all(dir.join("input/input5")).unwrap();
        fs::write(dir.join("input/input5/name"), "TPPS/2 Elan TrackPoint\n").unwrap();

        let node = node_at(&dir).unwrap();
        assert_eq!(node.input_names, vec!["TPPS/2 Elan TrackPoint"]);
    }

    #[test]
    fn a_node_without_an_input_directory_still_parses() {
        let dir = scratch("no-input");
        put(&dir, "sensitivity", "128");
        assert!(node_at(&dir).unwrap().input_names.is_empty());
    }

    #[test]
    fn shell_unsafe_values_are_refused_before_they_reach_a_shell() {
        let dir = scratch("injection");
        put(&dir, "sensitivity", "128");
        let mut node = node_at(&dir).unwrap();
        node.attrs[0].pending = "90; rm -rf /".into();
        node.attrs[0].writable_directly = false;
        assert!(write_attr(&node.attrs[0], Some("unused")).is_err());
    }
}
