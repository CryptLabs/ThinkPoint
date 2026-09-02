//! Thin wrapper around the `xinput` command line.
//!
//! Shelling out rather than linking against XInput2 keeps the dependency
//! footprint at zero and matches what a user would type by hand, which makes
//! the "here is the equivalent command" output in the UI honest.

use std::process::Command;

use crate::error::{Error, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    MasterPointer,
    SlavePointer,
    Other,
}

#[derive(Debug, Clone)]
pub struct XDevice {
    pub id: u32,
    pub name: String,
    pub kind: Kind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PropValue {
    /// Numbers we understand well enough to edit.
    Numbers(Vec<String>),
    /// Atoms, strings and anything else: displayed but not editable.
    Opaque(String),
}

#[derive(Debug, Clone)]
pub struct Prop {
    pub name: String,
    pub value: PropValue,
    /// True when xinput reports the property as read-only.
    pub read_only: bool,
}

impl Prop {
    /// A single 0/1 value, i.e. something a spacebar can toggle.
    pub fn as_bool(&self) -> Option<bool> {
        match &self.value {
            PropValue::Numbers(v) if v.len() == 1 && (v[0] == "0" || v[0] == "1") => {
                Some(v[0] == "1")
            }
            _ => None,
        }
    }

    /// A single floating point value, i.e. something the arrow keys can nudge.
    pub fn as_float(&self) -> Option<f32> {
        match &self.value {
            PropValue::Numbers(v) if v.len() == 1 && v[0].contains('.') => v[0].parse().ok(),
            _ => None,
        }
    }
}

fn run(args: &[&str]) -> Result<String> {
    let out = Command::new("xinput").args(args).output().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            Error::new("xinput not found — install xorg-xinput (X11 features disabled)")
        } else {
            Error::new(format!("xinput: {e}"))
        }
    })?;
    if !out.status.success() {
        let msg = String::from_utf8_lossy(&out.stderr).trim().to_string();
        let msg = if msg.is_empty() {
            format!("xinput {} failed", args.join(" "))
        } else {
            msg
        };
        return Err(Error::new(msg));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

pub fn available() -> bool {
    Command::new("xinput")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Parse the device tree printed by a bare `xinput list`.
///
/// Lines look like:
/// `⎜   ↳ TPPS/2 Elan TrackPoint            \tid=13\t[slave  pointer  (2)]`
pub fn list_pointers() -> Result<Vec<XDevice>> {
    let out = run(&["list"])?;
    let devices = parse_pointers(&out);
    if devices.is_empty() {
        return Err(Error::new("xinput reported no pointer devices"));
    }
    Ok(devices)
}

pub(crate) fn parse_pointers(out: &str) -> Vec<XDevice> {
    let mut devices = Vec::new();

    for line in out.lines() {
        let mut fields = line.split('\t');
        let Some(name_field) = fields.next() else {
            continue;
        };
        let Some(id_field) = fields.next() else {
            continue;
        };
        let type_field = fields.next().unwrap_or("");

        let Some(id) = id_field.trim().strip_prefix("id=") else {
            continue;
        };
        let Ok(id) = id.trim().parse::<u32>() else {
            continue;
        };

        let kind = if type_field.contains("slave") && type_field.contains("pointer") {
            Kind::SlavePointer
        } else if type_field.contains("master") && type_field.contains("pointer") {
            Kind::MasterPointer
        } else {
            Kind::Other
        };
        if kind == Kind::Other {
            continue;
        }

        let name = name_field
            .trim()
            .trim_start_matches(|c: char| {
                matches!(c, '⎡' | '⎜' | '⎣' | '⎢' | '⎥' | '⎦' | '↳' | '∼' | ' ')
            })
            .trim()
            .to_string();
        if name.is_empty() || name.contains("XTEST") {
            continue;
        }

        devices.push(XDevice { id, name, kind });
    }

    devices
}

pub fn get_button_map(id: u32) -> Result<Vec<u8>> {
    let out = run(&["get-button-map", &id.to_string()])?;
    let map: std::result::Result<Vec<u8>, _> =
        out.split_whitespace().map(|t| t.parse::<u8>()).collect();
    let map = map.map_err(|_| Error::new("could not parse the button map"))?;
    if map.is_empty() {
        return Err(Error::new("device reports no buttons"));
    }
    Ok(map)
}

pub fn set_button_map(id: u32, map: &[u8]) -> Result<()> {
    let mut args: Vec<String> = vec!["set-button-map".into(), id.to_string()];
    args.extend(map.iter().map(|b| b.to_string()));
    let refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    run(&refs).map(|_| ())
}

/// The command a user would type to reproduce the current map — shown in the
/// UI and written into generated scripts.
pub fn button_map_command(name: &str, map: &[u8]) -> String {
    let joined: Vec<String> = map.iter().map(|b| b.to_string()).collect();
    format!("xinput set-button-map \"{}\" {}", name, joined.join(" "))
}

/// Parse `xinput list-props`. Lines look like:
/// `\tlibinput Accel Speed (317):\t0.000000`
pub fn list_props(id: u32) -> Result<Vec<Prop>> {
    let out = run(&["list-props", &id.to_string()])?;
    Ok(parse_props(&out))
}

pub(crate) fn parse_props(out: &str) -> Vec<Prop> {
    let mut props = Vec::new();

    for line in out.lines() {
        if !line.starts_with('\t') {
            continue;
        }
        let mut fields = line.splitn(2, ":\t");
        let Some(head) = fields.next() else { continue };
        let value = fields.next().unwrap_or("").trim().to_string();

        let head = head.trim();
        let read_only = head.contains("(read-only)");
        let head = head.replace(" (read-only)", "");

        let Some(open) = head.rfind(" (") else {
            continue;
        };
        let name = head[..open].trim().to_string();
        // The trailing "(NNN)" is the property's atom id. We do not keep it —
        // xinput accepts names — but a line without one is not a property line.
        let Some(close) = head.rfind(')') else {
            continue;
        };
        if head[open + 2..close].parse::<u32>().is_err() {
            continue;
        }

        let parsed = if value.starts_with('"') || value.is_empty() {
            PropValue::Opaque(value)
        } else {
            let parts: Vec<String> = value.split(',').map(|s| s.trim().to_string()).collect();
            if parts.iter().all(|p| p.parse::<f64>().is_ok()) {
                PropValue::Numbers(parts)
            } else {
                PropValue::Opaque(value)
            }
        };

        props.push(Prop {
            name,
            value: parsed,
            read_only,
        });
    }

    props
}

/// Keep the property list to things worth putting in front of a user: the
/// libinput knobs, minus the `Default` and `Available` mirrors of each.
pub fn is_interesting(prop: &Prop) -> bool {
    if prop.read_only {
        return false;
    }
    if !prop.name.starts_with("libinput ") {
        return false;
    }
    if prop.name.ends_with("Default") || prop.name.contains("Available") {
        return false;
    }
    matches!(prop.value, PropValue::Numbers(_))
}

pub fn set_prop(id: u32, name: &str, values: &[String]) -> Result<()> {
    let mut args: Vec<String> = vec!["set-prop".into(), id.to_string(), name.to_string()];
    args.extend(values.iter().cloned());
    let refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    run(&refs).map(|_| ())
}

pub fn set_prop_command(name: &str, prop: &str, values: &[String]) -> String {
    format!(
        "xinput set-prop \"{}\" \"{}\" {}",
        name,
        prop,
        values.join(" ")
    )
}

/// The `/dev/input/eventN` node backing a device, used to find its sysfs home.
pub fn device_node(props: &[Prop]) -> Option<String> {
    props
        .iter()
        .find(|p| p.name == "Device Node")
        .and_then(|p| match &p.value {
            PropValue::Opaque(s) => Some(s.trim_matches('"').to_string()),
            _ => None,
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    // Real `xinput list` output from a ThinkPad, tabs and box-drawing
    // characters included, since those are what the parser has to survive.
    const LIST: &str = "\u{23a1} Virtual core pointer                     \tid=2\t[master pointer  (3)]\n\
        \u{239c}   \u{21b3} Virtual core XTEST pointer               \tid=4\t[slave  pointer  (2)]\n\
        \u{239c}   \u{21b3} SynPS/2 Synaptics TouchPad               \tid=12\t[slave  pointer  (2)]\n\
        \u{239c}   \u{21b3} TPPS/2 Elan TrackPoint                   \tid=13\t[slave  pointer  (2)]\n\
        \u{23a3} Virtual core keyboard                    \tid=3\t[master keyboard (2)]\n\
        \u{20}    \u{21b3} AT Translated Set 2 keyboard             \tid=14\t[slave  keyboard (3)]\n";

    #[test]
    fn parses_pointer_devices_and_drops_keyboards() {
        let devices = parse_pointers(LIST);
        let names: Vec<&str> = devices.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(
            names,
            vec![
                "Virtual core pointer",
                "SynPS/2 Synaptics TouchPad",
                "TPPS/2 Elan TrackPoint"
            ]
        );
    }

    #[test]
    fn drops_the_xtest_device() {
        assert!(
            !parse_pointers(LIST)
                .iter()
                .any(|d| d.name.contains("XTEST"))
        );
    }

    #[test]
    fn separates_masters_from_slaves() {
        let devices = parse_pointers(LIST);
        let trackpoint = devices
            .iter()
            .find(|d| d.name.contains("TrackPoint"))
            .unwrap();
        assert_eq!(trackpoint.id, 13);
        assert_eq!(trackpoint.kind, Kind::SlavePointer);
        assert_eq!(devices[0].kind, Kind::MasterPointer);
    }

    const PROPS: &str = "Device 'TPPS/2 Elan TrackPoint':\n\
        \tDevice Enabled (176):\t1\n\
        \tCoordinate Transformation Matrix (178):\t1.000000, 0.000000, 0.000000, 0.000000, 1.000000, 0.000000, 0.000000, 0.000000, 1.000000\n\
        \tlibinput Accel Speed (317):\t0.000000\n\
        \tlibinput Accel Speed Default (318):\t0.000000\n\
        \tlibinput Natural Scrolling Enabled (321):\t0\n\
        \tlibinput Scroll Methods Available (330):\t0, 0, 1\n\
        \tlibinput Middle Emulation Enabled (334):\t0\n\
        \tDevice Node (280):\t\"/dev/input/event10\"\n\
        \tDevice Product ID (281):\t2, 10\n";

    #[test]
    fn parses_property_names_and_values() {
        let props = parse_props(PROPS);
        let accel = props
            .iter()
            .find(|p| p.name == "libinput Accel Speed")
            .unwrap();
        assert_eq!(accel.as_float(), Some(0.0));

        let natural = props
            .iter()
            .find(|p| p.name == "libinput Natural Scrolling Enabled")
            .unwrap();
        assert_eq!(natural.as_bool(), Some(false));
    }

    #[test]
    fn hides_default_and_available_mirrors() {
        let props = parse_props(PROPS);
        let shown: Vec<&str> = props
            .iter()
            .filter(|p| is_interesting(p))
            .map(|p| p.name.as_str())
            .collect();
        assert_eq!(
            shown,
            vec![
                "libinput Accel Speed",
                "libinput Natural Scrolling Enabled",
                "libinput Middle Emulation Enabled"
            ]
        );
    }

    #[test]
    fn finds_the_device_node() {
        assert_eq!(
            device_node(&parse_props(PROPS)),
            Some("/dev/input/event10".to_string())
        );
    }

    #[test]
    fn quoted_strings_are_not_editable_numbers() {
        let props = parse_props(PROPS);
        let node = props.iter().find(|p| p.name == "Device Node").unwrap();
        assert!(matches!(node.value, PropValue::Opaque(_)));
    }

    #[test]
    fn renders_the_command_it_claims_to_run() {
        assert_eq!(
            button_map_command("TPPS/2 Elan TrackPoint", &[1, 0, 3, 4, 5, 6, 7]),
            "xinput set-button-map \"TPPS/2 Elan TrackPoint\" 1 0 3 4 5 6 7"
        );
    }
}
