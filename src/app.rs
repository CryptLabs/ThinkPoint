//! Application state and the actions the key bindings drive.

use crate::detect::{Detector, Meter};
use crate::error::{Error, Result};
use crate::persist::{self, XSetting};
use crate::sysfs::{self, Node, WriteVia};
use crate::xinput::{self, Kind, Prop, PropValue, XDevice};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Buttons,
    Libinput,
    Sysfs,
}

impl Tab {
    pub const ALL: [Tab; 3] = [Tab::Buttons, Tab::Libinput, Tab::Sysfs];

    pub fn title(self) -> &'static str {
        match self {
            Tab::Buttons => "Buttons",
            Tab::Libinput => "libinput",
            Tab::Sysfs => "sysfs",
        }
    }

    pub fn index(self) -> usize {
        match self {
            Tab::Buttons => 0,
            Tab::Libinput => 1,
            Tab::Sysfs => 2,
        }
    }

    pub fn next(self) -> Tab {
        Tab::ALL[(self.index() + 1) % Tab::ALL.len()]
    }

    pub fn prev(self) -> Tab {
        Tab::ALL[(self.index() + Tab::ALL.len() - 1) % Tab::ALL.len()]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Devices,
    Detail,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Info,
    Good,
    Warn,
    Bad,
}

#[derive(Debug, Clone)]
pub struct Status {
    pub text: String,
    pub level: Level,
}

impl Default for Status {
    fn default() -> Self {
        Status {
            text: "? for help  ·  q to quit".into(),
            level: Level::Info,
        }
    }
}

#[derive(Debug, Clone)]
pub struct PropRow {
    pub prop: Prop,
    pub pending: Vec<String>,
    pub original: Vec<String>,
}

impl PropRow {
    fn new(prop: Prop) -> Self {
        let values = match &prop.value {
            PropValue::Numbers(v) => v.clone(),
            PropValue::Opaque(s) => vec![s.clone()],
        };
        PropRow {
            prop,
            pending: values.clone(),
            original: values,
        }
    }

    pub fn is_dirty(&self) -> bool {
        match &self.prop.value {
            PropValue::Numbers(v) => &self.pending != v,
            PropValue::Opaque(_) => false,
        }
    }

    pub fn is_customised(&self) -> bool {
        self.pending != self.original
    }

    pub fn display(&self) -> String {
        self.pending.join(", ")
    }
}

#[derive(Debug, Clone)]
pub struct Device {
    pub x: Option<XDevice>,
    pub name: String,
    /// Live button map as last read from the X server.
    pub buttons: Vec<u8>,
    /// Staged map, written on apply.
    pub pending_buttons: Vec<u8>,
    /// Map as it was when the tool started.
    pub original_buttons: Vec<u8>,
    pub props: Vec<PropRow>,
    pub sysfs: Option<Node>,
    /// Whether the X server is currently delivering this device's events.
    pub enabled: bool,
    /// State at start-up, so the profile only carries deliberate changes.
    pub originally_enabled: bool,
    /// Set when a device could be listed but not fully interrogated.
    pub note: Option<String>,
}

impl Device {
    pub fn id(&self) -> Option<u32> {
        self.x.as_ref().map(|x| x.id)
    }

    pub fn buttons_dirty(&self) -> bool {
        self.pending_buttons != self.buttons
    }

    pub fn buttons_customised(&self) -> bool {
        self.pending_buttons != self.original_buttons
    }

    pub fn props_dirty(&self) -> bool {
        self.props.iter().any(|p| p.is_dirty())
    }

    pub fn sysfs_dirty(&self) -> bool {
        self.sysfs.as_ref().map(|n| n.dirty()).unwrap_or(false)
    }

    pub fn dirty(&self) -> bool {
        self.buttons_dirty() || self.props_dirty() || self.sysfs_dirty()
    }

    pub fn enabled_customised(&self) -> bool {
        self.enabled != self.originally_enabled
    }

    pub fn scroll_prop(&self) -> Option<usize> {
        self.props.iter().position(|p| p.prop.name == SCROLL_METHOD)
    }

    /// Whether libinput is currently set to scroll while the middle button is
    /// held.
    pub fn button_scrolling(&self) -> Option<bool> {
        let row = &self.props[self.scroll_prop()?];
        row.pending.get(SCROLL_BUTTON_INDEX).map(|v| v == "1")
    }

    /// Whether button 2 still reaches applications, which is what makes a
    /// middle click paste the primary selection.
    pub fn middle_click_delivered(&self) -> Option<bool> {
        self.pending_buttons.get(1).map(|b| *b != 0)
    }
}

pub enum Modal {
    Help,
    Detect(Box<Detector>),
    Meter(Box<Meter>),
    MiddleButton(MiddleButton),
    Password {
        buffer: String,
        /// What to retry once we have the password.
        action: RootAction,
        /// Set after a rejected attempt, so the second try says why.
        error: Option<String>,
    },
    Persist {
        rule: String,
        profile: String,
        /// 0 = udev rule, 1 = X profile.
        choice: usize,
    },
    Edit {
        title: String,
        buffer: String,
        target: EditTarget,
    },
}

/// The middle button does two unrelated jobs, and people usually want one of
/// them and not the other. They are also controlled in two different places:
/// pasting is the X button map delivering button 2 to applications, scrolling
/// is libinput consuming that button before the map is consulted. This gathers
/// both into one choice.
#[derive(Debug, Clone)]
pub struct MiddleButton {
    pub paste: bool,
    pub scroll: bool,
    /// 0 = paste row, 1 = scroll row.
    pub cursor: usize,
    pub paste_supported: bool,
    pub scroll_supported: bool,
}

/// Work that needs root, held while the password is collected.
#[derive(Debug, Clone)]
pub enum RootAction {
    /// Write the staged sysfs attributes on the selected device.
    Sysfs,
    /// Write the generated udev rule.
    Udev(String),
}

impl RootAction {
    pub fn describe(&self) -> String {
        match self {
            RootAction::Sysfs => "write the staged sysfs values".to_string(),
            RootAction::Udev(_) => format!("write {}", persist::UDEV_PATH),
        }
    }
}

/// The libinput property holding [two-finger, edge, button] scrolling.
pub const SCROLL_METHOD: &str = "libinput Scroll Method Enabled";
/// Index of the button-scrolling entry within it.
pub const SCROLL_BUTTON_INDEX: usize = 2;

#[derive(Debug, Clone, Copy)]
pub enum EditTarget {
    Prop(usize),
    Attr(usize),
}

pub struct App {
    pub devices: Vec<Device>,
    pub sel_device: usize,
    pub sel_button: usize,
    pub sel_prop: usize,
    pub sel_attr: usize,
    pub focus: Focus,
    pub tab: Tab,
    pub modal: Option<Modal>,
    pub status: Status,
    pub should_quit: bool,
    pub x11: bool,
}

/// What a button number conventionally does, so the UI can explain itself.
pub fn button_label(n: usize) -> &'static str {
    match n {
        1 => "left click",
        2 => "middle — primary-selection paste, TrackPoint scroll",
        3 => "right click",
        4 => "scroll up",
        5 => "scroll down",
        6 => "scroll left",
        7 => "scroll right",
        8 => "back",
        9 => "forward",
        _ => "",
    }
}

impl App {
    pub fn new() -> Result<Self> {
        let x11 = xinput::available();
        let mut devices = Vec::new();
        let mut claimed: Vec<std::path::PathBuf> = Vec::new();
        let mut note = None;

        if x11 {
            match xinput::list_pointers() {
                Ok(list) => {
                    for x in list.into_iter().filter(|d| d.kind == Kind::SlavePointer) {
                        let buttons = xinput::get_button_map(x.id).unwrap_or_default();
                        let all_props = xinput::list_props(x.id).unwrap_or_default();
                        let node =
                            xinput::device_node(&all_props).and_then(|n| sysfs::node_for_event(&n));
                        if let Some(n) = &node {
                            claimed.push(n.path.clone());
                        }
                        let enabled = xinput::is_enabled(&all_props);
                        let props: Vec<PropRow> = all_props
                            .into_iter()
                            .filter(xinput::is_interesting)
                            .map(PropRow::new)
                            .collect();

                        devices.push(Device {
                            name: x.name.clone(),
                            note: buttons
                                .is_empty()
                                .then(|| "no button map reported".to_string()),
                            buttons: buttons.clone(),
                            pending_buttons: buttons.clone(),
                            original_buttons: buttons,
                            props,
                            sysfs: node,
                            enabled,
                            originally_enabled: enabled,
                            x: Some(x),
                        });
                    }
                }
                Err(e) => note = Some(e.to_string()),
            }
        } else {
            note = Some("xinput unavailable — sysfs tuning only".to_string());
        }

        // Anything on the serio bus that no X device claimed still deserves a
        // row: under Wayland, that is the only view there is.
        //
        // "Claimed" cannot rest on the Device Node property alone. A disabled
        // device may stop reporting one, and then its serio node looks
        // unclaimed and the same touchpad appears twice — once as the real X
        // device and once as a dead sysfs-only row with no way to switch it
        // back on. Matching the node's own input names against the device list
        // closes that gap.
        let listed: Vec<String> = devices.iter().map(|d| d.name.clone()).collect();
        for node in sysfs::scan_all() {
            if claimed.contains(&node.path) {
                continue;
            }
            if node.input_names.iter().any(|n| listed.contains(n)) {
                continue;
            }
            devices.push(Device {
                x: None,
                name: node.label(),
                buttons: Vec::new(),
                pending_buttons: Vec::new(),
                original_buttons: Vec::new(),
                props: Vec::new(),
                sysfs: Some(node),
                enabled: true,
                originally_enabled: true,
                note: Some("sysfs only — not matched to an X device".to_string()),
            });
        }

        if devices.is_empty() {
            return Err(Error::new(
                "no pointer devices found via xinput or /sys/bus/serio",
            ));
        }

        let status = match note {
            Some(text) => Status {
                text,
                level: Level::Warn,
            },
            None => Status::default(),
        };

        Ok(App {
            devices,
            sel_device: 0,
            sel_button: 0,
            sel_prop: 0,
            sel_attr: 0,
            focus: Focus::Devices,
            tab: Tab::Buttons,
            modal: None,
            status,
            should_quit: false,
            x11,
        })
    }

    pub fn device(&self) -> &Device {
        &self.devices[self.sel_device]
    }

    fn device_mut(&mut self) -> &mut Device {
        &mut self.devices[self.sel_device]
    }

    pub fn say(&mut self, text: impl Into<String>, level: Level) {
        self.status = Status {
            text: text.into(),
            level,
        };
    }

    pub fn detail_len(&self) -> usize {
        match self.tab {
            Tab::Buttons => self.device().buttons.len(),
            Tab::Libinput => self.device().props.len(),
            Tab::Sysfs => self
                .device()
                .sysfs
                .as_ref()
                .map(|n| n.attrs.len())
                .unwrap_or(0),
        }
    }

    pub fn detail_cursor(&self) -> usize {
        match self.tab {
            Tab::Buttons => self.sel_button,
            Tab::Libinput => self.sel_prop,
            Tab::Sysfs => self.sel_attr,
        }
    }

    fn set_detail_cursor(&mut self, value: usize) {
        match self.tab {
            Tab::Buttons => self.sel_button = value,
            Tab::Libinput => self.sel_prop = value,
            Tab::Sysfs => self.sel_attr = value,
        }
    }

    pub fn move_cursor(&mut self, delta: isize) {
        match self.focus {
            Focus::Devices => {
                let len = self.devices.len();
                if len == 0 {
                    return;
                }
                let next = (self.sel_device as isize + delta).rem_euclid(len as isize) as usize;
                self.sel_device = next;
                self.sel_button = 0;
                self.sel_prop = 0;
                self.sel_attr = 0;
            }
            Focus::Detail => {
                let len = self.detail_len();
                if len == 0 {
                    return;
                }
                let next =
                    (self.detail_cursor() as isize + delta).rem_euclid(len as isize) as usize;
                self.set_detail_cursor(next);
            }
        }
    }

    // ---- Buttons -------------------------------------------------------

    pub fn toggle_button(&mut self) {
        let index = self.sel_button;
        let (message, level) = {
            let device = self.device_mut();
            if index >= device.pending_buttons.len() {
                return;
            }
            let physical = index + 1;
            if device.pending_buttons[index] == 0 {
                device.pending_buttons[index] = device.original_buttons[index];
                (
                    format!("Button {physical} restored (staged) — press a to apply"),
                    Level::Info,
                )
            } else {
                device.pending_buttons[index] = 0;
                if physical == 1 {
                    (
                        "Button 1 staged as disabled — that is your left click. \
                         Press space again to undo."
                            .to_string(),
                        Level::Warn,
                    )
                } else {
                    (
                        format!("Button {physical} staged as disabled — press a to apply"),
                        Level::Info,
                    )
                }
            }
        };
        self.say(message, level);
    }

    pub fn reset_buttons(&mut self) {
        let device = self.device_mut();
        device.pending_buttons = device.original_buttons.clone();
        self.say(
            "Button map reset to its start-up state (staged)",
            Level::Info,
        );
    }

    // ---- libinput properties -------------------------------------------

    pub fn adjust_prop(&mut self, delta: f32) {
        let index = self.sel_prop;
        let device = self.device_mut();
        let Some(row) = device.props.get_mut(index) else {
            return;
        };
        if let Some(current) = row.prop.as_float() {
            let base = row
                .pending
                .first()
                .and_then(|v| v.parse::<f32>().ok())
                .unwrap_or(current);
            let next = (base + delta).clamp(-1.0, 1.0);
            row.pending = vec![format!("{next:.6}")];
        } else if row.prop.as_bool().is_some() {
            let current = row.pending.first().map(|v| v == "1").unwrap_or(false);
            row.pending = vec![if current { "0" } else { "1" }.to_string()];
        }
    }

    pub fn toggle_prop(&mut self) {
        let index = self.sel_prop;
        let is_bool = self
            .device()
            .props
            .get(index)
            .map(|r| r.prop.as_bool().is_some())
            .unwrap_or(false);
        if !is_bool {
            self.say(
                "Not an on/off property — press e to edit its values",
                Level::Info,
            );
            return;
        }
        if let Some(row) = self.device_mut().props.get_mut(index) {
            let current = row.pending.first().map(|v| v == "1").unwrap_or(false);
            row.pending = vec![if current { "0" } else { "1" }.to_string()];
        }
    }

    // ---- sysfs ----------------------------------------------------------

    pub fn adjust_attr(&mut self, delta: i64) {
        let index = self.sel_attr;
        let device = self.device_mut();
        let Some(node) = device.sysfs.as_mut() else {
            return;
        };
        if let Some(attr) = node.attrs.get_mut(index) {
            attr.nudge(delta);
        }
    }

    pub fn toggle_attr(&mut self) {
        let index = self.sel_attr;
        let is_bool = self
            .device()
            .sysfs
            .as_ref()
            .and_then(|n| n.attrs.get(index))
            .map(|a| a.kind == sysfs::AttrKind::Bool)
            .unwrap_or(false);
        if !is_bool {
            self.say("Use ← / → to adjust, or e to type a value", Level::Info);
            return;
        }
        if let Some(node) = self.device_mut().sysfs.as_mut()
            && let Some(attr) = node.attrs.get_mut(index)
        {
            attr.toggle();
        }
    }

    // ---- Editing --------------------------------------------------------

    pub fn open_editor(&mut self) {
        match self.tab {
            Tab::Libinput => {
                let index = self.sel_prop;
                let Some(row) = self.device().props.get(index) else {
                    return;
                };
                self.modal = Some(Modal::Edit {
                    title: row.prop.name.clone(),
                    buffer: row.pending.join(" "),
                    target: EditTarget::Prop(index),
                });
            }
            Tab::Sysfs => {
                let index = self.sel_attr;
                let Some(attr) = self
                    .device()
                    .sysfs
                    .as_ref()
                    .and_then(|n| n.attrs.get(index))
                else {
                    return;
                };
                self.modal = Some(Modal::Edit {
                    title: attr.name.clone(),
                    buffer: attr.pending.clone(),
                    target: EditTarget::Attr(index),
                });
            }
            Tab::Buttons => {
                self.say("Nothing to type here — space toggles a button", Level::Info);
            }
        }
    }

    pub fn commit_editor(&mut self, buffer: String, target: EditTarget) {
        match target {
            EditTarget::Prop(index) => {
                let values: Vec<String> =
                    buffer.split_whitespace().map(|s| s.to_string()).collect();
                if values.is_empty() || values.iter().any(|v| v.parse::<f64>().is_err()) {
                    self.say("Expected one or more numbers", Level::Bad);
                    return;
                }
                let expected = match self.device().props.get(index).map(|r| &r.prop.value) {
                    Some(PropValue::Numbers(current)) => Some(current.len()),
                    _ => None,
                };
                if let Some(expected) = expected
                    && expected != values.len()
                {
                    self.say(
                        format!("That property takes {expected} value(s)"),
                        Level::Bad,
                    );
                    return;
                }
                if let Some(row) = self.device_mut().props.get_mut(index) {
                    row.pending = values;
                }
            }
            EditTarget::Attr(index) => {
                let trimmed = buffer.trim().to_string();
                if trimmed.parse::<i64>().is_err() {
                    self.say("Expected a whole number", Level::Bad);
                    return;
                }
                let device = self.device_mut();
                if let Some(node) = device.sysfs.as_mut()
                    && let Some(attr) = node.attrs.get_mut(index)
                {
                    attr.pending = trimmed;
                    attr.nudge(0); // clamp into range
                }
            }
        }
        self.say("Staged — press a to apply", Level::Info);
    }

    // ---- Applying -------------------------------------------------------

    pub fn apply(&mut self) {
        self.apply_with(None);
    }

    /// `password` is only ever Some when coming back from the prompt, and is
    /// used for this one call before the caller scrubs it.
    pub fn apply_with(&mut self, password: Option<&str>) {
        let result = match self.tab {
            Tab::Buttons => self.apply_buttons(),
            Tab::Libinput => self.apply_props(),
            Tab::Sysfs => self.apply_sysfs(password),
        };
        match result {
            Ok(message) => self.say(message, Level::Good),
            Err(e) if e.is_needs_password() => self.ask_for_password(RootAction::Sysfs, None),
            Err(e) => self.say(e.to_string(), Level::Bad),
        }
    }

    pub fn ask_for_password(&mut self, action: RootAction, error: Option<String>) {
        self.modal = Some(Modal::Password {
            buffer: String::new(),
            action,
            error,
        });
    }

    /// Retry whatever needed root, now that we have a password. The caller owns
    /// the string and scrubs it immediately afterwards.
    pub fn retry_with_password(&mut self, action: RootAction, password: &str) {
        let result = match &action {
            RootAction::Sysfs => self.apply_sysfs(Some(password)),
            RootAction::Udev(rule) => {
                let rule = rule.clone();
                sysfs::write_root_file(persist::UDEV_PATH, &rule, Some(password)).map(|_| {
                    format!(
                        "Wrote {} — reload with: sudo udevadm control --reload",
                        persist::UDEV_PATH
                    )
                })
            }
        };
        match result {
            Ok(message) => self.say(message, Level::Good),
            Err(e) if e.is_needs_password() => self.ask_for_password(action, None),
            Err(e) => {
                // A wrong password should reopen the prompt rather than dump the
                // user back to the main screen to start again.
                let message = e.to_string();
                if message.contains("not accepted") {
                    self.ask_for_password(action, Some(message));
                } else {
                    self.say(message, Level::Bad);
                }
            }
        }
    }

    fn apply_buttons(&mut self) -> Result<String> {
        let device = self.device();
        if !device.buttons_dirty() {
            return Ok("Button map already matches what is staged".into());
        }
        let id = device
            .id()
            .ok_or_else(|| Error::new("this device has no X id"))?;
        let map = device.pending_buttons.clone();
        xinput::set_button_map(id, &map)?;

        let confirmed = xinput::get_button_map(id)?;
        let device = self.device_mut();
        device.buttons = confirmed.clone();
        if confirmed != map {
            return Err(Error::new(
                "the X server accepted the call but kept the old map",
            ));
        }
        let disabled: Vec<String> = map
            .iter()
            .enumerate()
            .filter(|(_, b)| **b == 0)
            .map(|(i, _)| (i + 1).to_string())
            .collect();
        Ok(if disabled.is_empty() {
            "Button map applied — all buttons active".into()
        } else {
            format!("Button map applied — disabled: {}", disabled.join(", "))
        })
    }

    fn apply_props(&mut self) -> Result<String> {
        let device = self.device();
        let id = device
            .id()
            .ok_or_else(|| Error::new("this device has no X id"))?;
        let changes: Vec<(String, Vec<String>)> = device
            .props
            .iter()
            .filter(|r| r.is_dirty())
            .map(|r| (r.prop.name.clone(), r.pending.clone()))
            .collect();
        if changes.is_empty() {
            return Ok("No staged property changes".into());
        }
        for (name, values) in &changes {
            xinput::set_prop(id, name, values)?;
        }

        let refreshed = xinput::list_props(id)?;
        let device = self.device_mut();
        for row in device.props.iter_mut() {
            if let Some(updated) = refreshed.iter().find(|p| p.name == row.prop.name) {
                row.prop = updated.clone();
            }
        }
        Ok(format!("Applied {} property change(s)", changes.len()))
    }

    fn apply_sysfs(&mut self, password: Option<&str>) -> Result<String> {
        let device = self.device_mut();
        let Some(node) = device.sysfs.as_mut() else {
            return Err(Error::new("this device has no sysfs attributes"));
        };
        let dirty: Vec<usize> = node
            .attrs
            .iter()
            .enumerate()
            .filter(|(_, a)| a.is_dirty())
            .map(|(i, _)| i)
            .collect();
        if dirty.is_empty() {
            return Ok("No staged sysfs changes".into());
        }

        let mut elevated = false;
        for index in &dirty {
            let attr = &node.attrs[*index];
            match sysfs::write_attr(attr, password) {
                Ok(WriteVia::SudoCached | WriteVia::SudoPassword) => elevated = true,
                Ok(WriteVia::Direct) => {}
                Err(e) => return Err(e),
            }
            let written = attr.pending.clone();
            node.attrs[*index].value = written;
        }

        Ok(format!(
            "Wrote {} sysfs attribute(s){}",
            dirty.len(),
            if elevated { " as root" } else { "" }
        ))
    }

    // ---- Persistence ----------------------------------------------------

    pub fn x_settings(&self) -> Vec<XSetting> {
        self.devices
            .iter()
            .filter(|d| d.x.is_some())
            .filter(|d| {
                d.buttons_customised()
                    || d.enabled_customised()
                    || d.props.iter().any(|p| p.is_customised())
            })
            .map(|d| XSetting {
                device: d.name.clone(),
                enabled: d.enabled_customised().then_some(d.enabled),
                button_map: d.buttons_customised().then(|| d.pending_buttons.clone()),
                props: d
                    .props
                    .iter()
                    .filter(|p| p.is_customised())
                    .map(|p| (p.prop.name.clone(), p.pending.clone()))
                    .collect(),
            })
            .collect()
    }

    pub fn open_persist(&mut self) {
        let nodes: Vec<Node> = self
            .devices
            .iter()
            .filter_map(|d| d.sysfs.clone())
            .collect();
        let rule = persist::udev_rule(&nodes, true);
        let profile = persist::render_profile(&self.x_settings());
        self.modal = Some(Modal::Persist {
            rule,
            profile,
            choice: 0,
        });
    }

    pub fn write_udev(&mut self, rule: &str) {
        match sysfs::write_root_file(persist::UDEV_PATH, rule, None) {
            Ok(_) => self.say(
                format!(
                    "Wrote {} — reload with: sudo udevadm control --reload",
                    persist::UDEV_PATH
                ),
                Level::Good,
            ),
            Err(e) if e.is_needs_password() => {
                self.ask_for_password(RootAction::Udev(rule.to_string()), None)
            }
            Err(e) => self.say(e.to_string(), Level::Bad),
        }
    }

    pub fn write_profile(&mut self) {
        match persist::save_profile(&self.x_settings()) {
            Ok(path) => self.say(
                format!(
                    "Wrote {} — replay with: thinkpoint --restore",
                    path.display()
                ),
                Level::Good,
            ),
            Err(e) => self.say(e.to_string(), Level::Bad),
        }
    }

    // ---- Detector -------------------------------------------------------

    pub fn open_detector(&mut self) {
        if !self.x11 {
            self.say("Needs xinput, which is not available here", Level::Bad);
            return;
        }
        match Detector::start() {
            Ok(d) => {
                self.modal = Some(Modal::Detect(Box::new(d)));
                self.say("Click a button to see which device sends it", Level::Info);
            }
            Err(e) => self.say(e.to_string(), Level::Bad),
        }
    }

    /// Turn the selected device off or on.
    ///
    /// Unlike the value editors this applies immediately: it is one call, it is
    /// undone by the same key, and staging a device's existence would be a
    /// strange thing to make someone confirm. Disabling leaves every other
    /// setting on the device untouched.
    pub fn toggle_device_enabled(&mut self) {
        let index = self.sel_device;
        let Some(id) = self.devices[index].id() else {
            self.say(
                "This row has no X device, so there is nothing to enable or disable",
                Level::Bad,
            );
            return;
        };

        // Read the live state rather than trusting what was cached at start-up.
        // If that value were stale — the device disabled by an earlier run, say
        // — the toggle would invert and appear to do nothing at all.
        let current = match xinput::read_enabled(id) {
            Ok(state) => {
                self.devices[index].enabled = state;
                state
            }
            Err(e) => {
                self.say(
                    format!("could not read the device's state: {e}"),
                    Level::Bad,
                );
                return;
            }
        };
        let target = !current;

        if let Err(e) = xinput::set_enabled(id, target) {
            self.say(e.to_string(), Level::Bad);
            return;
        }

        // And confirm it took, rather than reporting success on the strength of
        // an exit code.
        let confirmed = xinput::read_enabled(id).unwrap_or(target);
        self.devices[index].enabled = confirmed;
        let name = self.devices[index].name.clone();

        if confirmed != target {
            self.say(
                format!("{name} did not change state — the X server kept it as it was"),
                Level::Bad,
            );
        } else if confirmed {
            self.say(format!("{name} enabled"), Level::Good);
        } else {
            self.say(
                format!("{name} disabled — press t again to bring it back"),
                Level::Warn,
            );
        }
    }

    pub fn open_middle_button(&mut self) {
        let device = self.device();
        if device.id().is_none() {
            self.say("This row has no X device to configure", Level::Bad);
            return;
        }
        let paste_supported = device.middle_click_delivered().is_some();
        let scroll_supported = device.button_scrolling().is_some();

        if !paste_supported && !scroll_supported {
            self.say(
                "This device reports neither a middle button nor a scroll method",
                Level::Bad,
            );
            return;
        }

        self.modal = Some(Modal::MiddleButton(MiddleButton {
            paste: device.middle_click_delivered().unwrap_or(false),
            scroll: device.button_scrolling().unwrap_or(false),
            cursor: 0,
            paste_supported,
            scroll_supported,
        }));
    }

    /// Apply both halves of the middle-button choice at once, since leaving one
    /// staged and the other live is exactly the confusion this screen exists to
    /// remove.
    pub fn apply_middle_button(&mut self, choice: MiddleButton) {
        let index = self.sel_device;
        let Some(id) = self.devices[index].id() else {
            return;
        };
        let mut done: Vec<String> = Vec::new();

        if choice.paste_supported {
            let restore = self.devices[index]
                .original_buttons
                .get(1)
                .copied()
                .filter(|b| *b != 0)
                .unwrap_or(2);
            let target = if choice.paste { restore } else { 0 };
            if self.devices[index].pending_buttons.get(1) != Some(&target) {
                self.devices[index].pending_buttons[1] = target;
            }
            if self.devices[index].buttons_dirty() {
                let map = self.devices[index].pending_buttons.clone();
                if let Err(e) = xinput::set_button_map(id, &map) {
                    self.say(e.to_string(), Level::Bad);
                    return;
                }
                match xinput::get_button_map(id) {
                    Ok(confirmed) => {
                        let took = confirmed == map;
                        self.devices[index].buttons = confirmed;
                        if !took {
                            self.say(
                                "the X server accepted the button map but kept the old one",
                                Level::Bad,
                            );
                            return;
                        }
                    }
                    Err(e) => {
                        self.say(e.to_string(), Level::Bad);
                        return;
                    }
                }
            }
            done.push(format!("paste {}", if choice.paste { "on" } else { "off" }));
        }

        if choice.scroll_supported
            && let Some(row_index) = self.devices[index].scroll_prop()
        {
            let mut values = self.devices[index].props[row_index].pending.clone();
            if values.len() > SCROLL_BUTTON_INDEX {
                values[SCROLL_BUTTON_INDEX] = if choice.scroll { "1" } else { "0" }.to_string();
                if let Err(e) = xinput::set_prop(id, SCROLL_METHOD, &values) {
                    self.say(e.to_string(), Level::Bad);
                    return;
                }
                self.devices[index].props[row_index].pending = values.clone();
                if let PropValue::Numbers(live) =
                    &mut self.devices[index].props[row_index].prop.value
                {
                    *live = values;
                }
                done.push(format!(
                    "scroll {}",
                    if choice.scroll { "on" } else { "off" }
                ));
            }
        }

        if done.is_empty() {
            self.say("Nothing to change", Level::Info);
        } else {
            self.say(format!("Middle button: {}", done.join(", ")), Level::Good);
        }
    }

    pub fn open_meter(&mut self) {
        if !self.x11 {
            self.say("Needs xinput, which is not available here", Level::Bad);
            return;
        }
        match Meter::start() {
            Ok(m) => {
                self.modal = Some(Modal::Meter(Box::new(m)));
                self.say("Take your hands off the machine and watch", Level::Info);
            }
            Err(e) => self.say(e.to_string(), Level::Bad),
        }
    }

    // ---- Drift preset ---------------------------------------------------

    /// Stage the settings that reduce drift, and say plainly which of them this
    /// device can actually take.
    ///
    /// Lowering `sensitivity` does not stop the underlying creep — it scales
    /// the motion the same spurious force produces, which is usually enough to
    /// stop it being noticeable. `drift_time` is the real correction knob, and
    /// the kernel only exposes it on IBM TrackPoints, so on an Elan, ALPS or
    /// NXP stick there is nothing here to tune and the honest answer is to say
    /// so rather than to pretend the preset did more than it did.
    pub fn apply_drift_preset(&mut self) {
        let index = self.sel_device;
        let Some(node) = self.devices[index].sysfs.as_mut() else {
            self.say(
                "No sysfs node on this device, so no kernel-side drift settings",
                Level::Bad,
            );
            return;
        };

        let mut staged: Vec<String> = Vec::new();
        let mut had_sensitivity = false;

        for attr in node.attrs.iter_mut() {
            match attr.name.as_str() {
                "sensitivity" => {
                    had_sensitivity = true;
                    let current = attr.as_int().unwrap_or(128);
                    // Three quarters, floored at 40: below that the stick gets
                    // unusable well before the drift stops mattering.
                    let target = ((current * 3) / 4).max(40);
                    if target != current {
                        attr.pending = target.to_string();
                        staged.push(format!("sensitivity {current} → {target}"));
                    }
                }
                "drift_time" => {
                    let current = attr.as_int().unwrap_or(5);
                    if current < 20 {
                        attr.pending = "20".to_string();
                        staged.push(format!("drift_time {current} → 20"));
                    }
                }
                _ => {}
            }
        }

        let has_drift_time = node.attrs.iter().any(|a| a.name == "drift_time");
        self.tab = Tab::Sysfs;
        self.focus = Focus::Detail;

        if staged.is_empty() {
            let why = if had_sensitivity {
                "Already at or below the preset's values"
            } else {
                "This device exposes no drift-related attributes"
            };
            self.say(why, Level::Warn);
            return;
        }

        let note = if has_drift_time {
            String::new()
        } else {
            "  ·  no drift_time on this device, so firmware drift correction cannot be tuned"
                .to_string()
        };
        self.say(
            format!("Staged: {}{note}  ·  press a to apply", staged.join(", ")),
            Level::Good,
        );
    }

    pub fn device_name_for_id(&self, id: u32) -> String {
        self.devices
            .iter()
            .find(|d| d.id() == Some(id))
            .map(|d| d.name.clone())
            .unwrap_or_else(|| format!("device {id} (not in this list)"))
    }

    pub fn refresh(&mut self) {
        match App::new() {
            Ok(fresh) => {
                let selected = self.sel_device.min(fresh.devices.len().saturating_sub(1));
                let tab = self.tab;
                *self = fresh;
                self.sel_device = selected;
                self.tab = tab;
                self.say("Reloaded from the system", Level::Good);
            }
            Err(e) => self.say(e.to_string(), Level::Bad),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sysfs::{Attr, AttrKind, Node};
    use crate::xinput::{Prop, PropValue};
    use std::path::PathBuf;

    fn attr(name: &str, value: &str, kind: AttrKind) -> Attr {
        Attr {
            name: name.to_string(),
            help: "",
            kind,
            value: value.to_string(),
            original: value.to_string(),
            pending: value.to_string(),
            writable_directly: false,
            path: PathBuf::from("/sys/devices/platform/i8042/serio1").join(name),
        }
    }

    fn device(attrs: Vec<Attr>) -> Device {
        Device {
            x: None,
            name: "TPPS/2 Elan TrackPoint".into(),
            buttons: vec![1, 2, 3],
            pending_buttons: vec![1, 2, 3],
            original_buttons: vec![1, 2, 3],
            props: Vec::new(),
            enabled: true,
            originally_enabled: true,
            sysfs: (!attrs.is_empty()).then(|| Node {
                path: PathBuf::from("/sys/devices/platform/i8042/serio1"),
                description: String::new(),
                firmware_id: String::new(),
                input_names: Vec::new(),
                attrs,
            }),
            note: None,
        }
    }

    fn app(attrs: Vec<Attr>) -> App {
        App {
            devices: vec![device(attrs)],
            sel_device: 0,
            sel_button: 0,
            sel_prop: 0,
            sel_attr: 0,
            focus: Focus::Devices,
            tab: Tab::Buttons,
            modal: None,
            status: Status::default(),
            should_quit: false,
            x11: true,
        }
    }

    fn pending(app: &App, name: &str) -> Option<String> {
        app.device()
            .sysfs
            .as_ref()?
            .attrs
            .iter()
            .find(|a| a.name == name)
            .map(|a| a.pending.clone())
    }

    #[test]
    fn drift_preset_lowers_sensitivity_by_a_quarter() {
        let mut a = app(vec![attr("sensitivity", "128", AttrKind::Range(0, 255))]);
        a.apply_drift_preset();
        assert_eq!(pending(&a, "sensitivity").as_deref(), Some("96"));
        assert_eq!(a.status.level, Level::Good);
    }

    #[test]
    fn drift_preset_stops_before_the_stick_becomes_unusable() {
        let mut a = app(vec![attr("sensitivity", "45", AttrKind::Range(0, 255))]);
        a.apply_drift_preset();
        assert_eq!(pending(&a, "sensitivity").as_deref(), Some("40"));
    }

    #[test]
    fn drift_preset_raises_drift_time_where_the_device_has_one() {
        let mut a = app(vec![
            attr("sensitivity", "128", AttrKind::Range(0, 255)),
            attr("drift_time", "5", AttrKind::Range(0, 255)),
        ]);
        a.apply_drift_preset();
        assert_eq!(pending(&a, "drift_time").as_deref(), Some("20"));
        assert!(!a.status.text.contains("no drift_time"));
    }

    #[test]
    fn drift_preset_says_so_when_there_is_no_drift_time() {
        // The Elan case: sensitivity is the only lever the kernel offers.
        let mut a = app(vec![attr("sensitivity", "128", AttrKind::Range(0, 255))]);
        a.apply_drift_preset();
        assert!(
            a.status.text.contains("no drift_time on this device"),
            "should not imply it tuned something it cannot: {}",
            a.status.text
        );
    }

    #[test]
    fn drift_preset_stages_rather_than_writes() {
        let mut a = app(vec![attr("sensitivity", "128", AttrKind::Range(0, 255))]);
        a.apply_drift_preset();
        let node = a.device().sysfs.as_ref().unwrap();
        let sensitivity = &node.attrs[0];
        assert!(sensitivity.is_dirty(), "staged");
        assert_eq!(
            sensitivity.value, "128",
            "kernel value untouched until apply"
        );
        assert!(a.status.text.contains("press a to apply"));
    }

    #[test]
    fn drift_preset_moves_you_to_the_tab_showing_the_change() {
        let mut a = app(vec![attr("sensitivity", "128", AttrKind::Range(0, 255))]);
        a.apply_drift_preset();
        assert_eq!(a.tab, Tab::Sysfs);
    }

    #[test]
    fn drift_preset_is_idempotent_at_the_floor() {
        let mut a = app(vec![attr("sensitivity", "40", AttrKind::Range(0, 255))]);
        a.apply_drift_preset();
        assert_eq!(a.status.level, Level::Warn);
        assert!(a.status.text.contains("Already at or below"));
    }

    #[test]
    fn a_disabled_device_is_carried_into_the_profile() {
        let mut a = app(vec![attr("sensitivity", "128", AttrKind::Range(0, 255))]);
        a.devices[0].x = Some(crate::xinput::XDevice {
            id: 12,
            name: "SynPS/2 Synaptics TouchPad".into(),
            kind: crate::xinput::Kind::SlavePointer,
        });
        a.devices[0].name = "SynPS/2 Synaptics TouchPad".into();

        assert!(a.x_settings().is_empty(), "nothing customised yet");

        // Simulate the toggle having gone through, without invoking xinput.
        a.devices[0].enabled = false;
        let settings = a.x_settings();
        assert_eq!(settings.len(), 1);
        assert_eq!(settings[0].enabled, Some(false));
    }

    #[test]
    fn a_device_left_enabled_is_not_carried_into_the_profile() {
        let mut a = app(vec![attr("sensitivity", "128", AttrKind::Range(0, 255))]);
        a.devices[0].x = Some(crate::xinput::XDevice {
            id: 12,
            name: "Some Mouse".into(),
            kind: crate::xinput::Kind::SlavePointer,
        });
        assert!(a.x_settings().is_empty());
    }

    fn with_scroll(paste_disabled: bool, scroll_on: bool) -> App {
        let mut a = app(vec![attr("sensitivity", "128", AttrKind::Range(0, 255))]);
        let d = &mut a.devices[0];
        d.x = Some(crate::xinput::XDevice {
            id: 13,
            name: "TPPS/2 Elan TrackPoint".into(),
            kind: crate::xinput::Kind::SlavePointer,
        });
        d.buttons = vec![1, 2, 3, 4, 5, 6, 7];
        d.original_buttons = vec![1, 2, 3, 4, 5, 6, 7];
        d.pending_buttons = if paste_disabled {
            vec![1, 0, 3, 4, 5, 6, 7]
        } else {
            vec![1, 2, 3, 4, 5, 6, 7]
        };
        let values = vec![
            "0".to_string(),
            "0".to_string(),
            if scroll_on { "1" } else { "0" }.to_string(),
        ];
        d.props = vec![PropRow {
            prop: Prop {
                name: SCROLL_METHOD.to_string(),
                value: PropValue::Numbers(values.clone()),
                read_only: false,
            },
            pending: values.clone(),
            original: values,
        }];
        a
    }

    #[test]
    fn middle_button_reads_the_current_paste_and_scroll_state() {
        let a = with_scroll(true, true);
        assert_eq!(a.device().middle_click_delivered(), Some(false));
        assert_eq!(a.device().button_scrolling(), Some(true));

        let a = with_scroll(false, false);
        assert_eq!(a.device().middle_click_delivered(), Some(true));
        assert_eq!(a.device().button_scrolling(), Some(false));
    }

    #[test]
    fn middle_button_panel_opens_with_the_live_values() {
        let mut a = with_scroll(true, true);
        a.open_middle_button();
        match a.modal {
            Some(Modal::MiddleButton(ref choice)) => {
                assert!(!choice.paste, "paste is currently off");
                assert!(choice.scroll, "scroll is currently on");
                assert!(choice.paste_supported && choice.scroll_supported);
            }
            _ => panic!("expected the middle button panel"),
        }
    }

    #[test]
    fn a_device_with_neither_capability_gets_no_panel() {
        let mut a = app(vec![attr("sensitivity", "128", AttrKind::Range(0, 255))]);
        a.devices[0].x = Some(crate::xinput::XDevice {
            id: 9,
            name: "Odd Device".into(),
            kind: crate::xinput::Kind::SlavePointer,
        });
        a.devices[0].buttons = Vec::new();
        a.devices[0].pending_buttons = Vec::new();
        a.devices[0].props = Vec::new();
        a.open_middle_button();
        assert!(a.modal.is_none());
        assert_eq!(a.status.level, Level::Bad);
    }

    #[test]
    fn scroll_and_paste_are_independent_settings() {
        // The whole point: they live in different places, so one can be on
        // while the other is off, in any combination.
        for (paste, scroll) in [(true, true), (true, false), (false, true), (false, false)] {
            let mut a = with_scroll(!paste, scroll);
            a.open_middle_button();
            let Some(Modal::MiddleButton(choice)) = a.modal.as_ref() else {
                panic!("no panel");
            };
            assert_eq!(choice.paste, paste);
            assert_eq!(choice.scroll, scroll);
        }
    }

    #[test]
    fn toggling_a_row_with_no_x_device_says_why() {
        let mut a = app(vec![attr("sensitivity", "128", AttrKind::Range(0, 255))]);
        a.toggle_device_enabled();
        assert_eq!(a.status.level, Level::Bad);
        assert!(a.status.text.contains("no X device"));
    }

    #[test]
    fn drift_preset_on_a_device_without_sysfs_explains_itself() {
        let mut a = app(Vec::new());
        a.apply_drift_preset();
        assert_eq!(a.status.level, Level::Bad);
        assert!(a.status.text.contains("No sysfs node"));
    }
}
