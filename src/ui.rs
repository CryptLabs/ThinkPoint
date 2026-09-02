//! Rendering. Nothing here mutates state except the detector's poll, which is
//! a drain of an already-filled channel.

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, BorderType, Borders, Clear, List, ListItem, ListState, Paragraph, Tabs, Wrap,
};

use crate::app::{App, Focus, Level, Modal, Tab, button_label};
use crate::sysfs::AttrKind;

const ACCENT: Color = Color::Cyan;
const DIM: Color = Color::DarkGray;
const PENDING: Color = Color::Yellow;
const OFF: Color = Color::Red;

fn focused_border(focused: bool) -> Style {
    if focused {
        Style::default().fg(ACCENT)
    } else {
        Style::default().fg(DIM)
    }
}

fn panel(title: &str, focused: bool) -> Block<'_> {
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(focused_border(focused))
        .title(Span::styled(
            format!(" {title} "),
            Style::default()
                .fg(if focused { ACCENT } else { Color::Gray })
                .add_modifier(Modifier::BOLD),
        ))
}

pub fn draw(frame: &mut Frame, app: &mut App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(6),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .split(frame.area());

    draw_title(frame, chunks[0]);

    let body = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(36), Constraint::Percentage(64)])
        .split(chunks[1]);

    draw_devices(frame, app, body[0]);
    draw_detail(frame, app, body[1]);
    draw_status(frame, app, chunks[2]);
    draw_keys(frame, app, chunks[3]);

    if app.modal.is_some() {
        draw_modal(frame, app);
    }
}

fn draw_title(frame: &mut Frame, area: Rect) {
    let line = Line::from(vec![
        Span::styled(
            " ThinkPoint ",
            Style::default()
                .fg(Color::Black)
                .bg(ACCENT)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            "  buttons, libinput and TrackPoint tuning",
            Style::default().fg(DIM),
        ),
    ]);
    frame.render_widget(Paragraph::new(line), area);
}

fn draw_devices(frame: &mut Frame, app: &App, area: Rect) {
    let focused = app.focus == Focus::Devices;
    let items: Vec<ListItem> = app
        .devices
        .iter()
        .map(|d| {
            let mut spans = vec![Span::raw(d.name.clone())];
            if d.dirty() {
                spans.push(Span::styled(" ●", Style::default().fg(PENDING)));
            }
            let mut lines = vec![Line::from(spans)];
            let mut tags: Vec<String> = Vec::new();
            if let Some(id) = d.id() {
                tags.push(format!("id {id}"));
            }
            if !d.buttons.is_empty() {
                tags.push(format!("{} buttons", d.buttons.len()));
            }
            if d.sysfs.is_some() {
                tags.push("sysfs".into());
            }
            lines.push(Line::from(Span::styled(
                format!("  {}", tags.join(" · ")),
                Style::default().fg(DIM),
            )));
            ListItem::new(lines)
        })
        .collect();

    let list = List::new(items)
        .block(panel("Devices", focused))
        .highlight_style(
            Style::default()
                .fg(Color::Black)
                .bg(if focused { ACCENT } else { Color::Gray })
                .add_modifier(Modifier::BOLD),
        );
    let mut state = ListState::default().with_selected(Some(app.sel_device));
    frame.render_stateful_widget(list, area, &mut state);
}

fn draw_detail(frame: &mut Frame, app: &App, area: Rect) {
    let focused = app.focus == Focus::Detail;
    let outer = panel(&app.device().name, focused);
    let inner = outer.inner(area);
    frame.render_widget(outer, area);

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(3),
            Constraint::Length(3),
        ])
        .split(inner);

    let titles: Vec<Line> = Tab::ALL
        .iter()
        .map(|t| Line::from(t.title().to_string()))
        .collect();
    let tabs = Tabs::new(titles)
        .select(app.tab.index())
        .style(Style::default().fg(DIM))
        .highlight_style(
            Style::default()
                .fg(ACCENT)
                .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
        )
        .divider(Span::styled("│", Style::default().fg(DIM)));
    frame.render_widget(tabs, rows[0]);

    match app.tab {
        Tab::Buttons => draw_buttons(frame, app, rows[1], focused),
        Tab::Libinput => draw_props(frame, app, rows[1], focused),
        Tab::Sysfs => draw_attrs(frame, app, rows[1], focused),
    }

    draw_hint(frame, app, rows[2]);
}

fn empty(frame: &mut Frame, area: Rect, message: &str) {
    frame.render_widget(
        Paragraph::new(message)
            .style(Style::default().fg(DIM))
            .wrap(Wrap { trim: true }),
        area,
    );
}

fn draw_buttons(frame: &mut Frame, app: &App, area: Rect, focused: bool) {
    let device = app.device();
    if device.buttons.is_empty() {
        empty(
            frame,
            area,
            device
                .note
                .clone()
                .unwrap_or_else(|| "This device reports no buttons.".into())
                .as_str(),
        );
        return;
    }

    let items: Vec<ListItem> = device
        .pending_buttons
        .iter()
        .enumerate()
        .map(|(i, target)| {
            let physical = i + 1;
            let live = device.buttons.get(i).copied().unwrap_or(*target);
            let changed = *target != live;

            let target_span = if *target == 0 {
                Span::styled(
                    "disabled".to_string(),
                    Style::default().fg(OFF).add_modifier(Modifier::BOLD),
                )
            } else {
                Span::styled(target.to_string(), Style::default().fg(Color::Green))
            };

            let mut spans = vec![
                Span::styled(
                    format!("{physical:>3}  →  "),
                    Style::default().fg(Color::Gray),
                ),
                target_span,
            ];
            if changed {
                spans.push(Span::styled(
                    format!("   (staged, live: {live})"),
                    Style::default().fg(PENDING),
                ));
            }
            let label = button_label(physical);
            if !label.is_empty() {
                spans.push(Span::styled(
                    format!("   {label}"),
                    Style::default().fg(DIM),
                ));
            }
            ListItem::new(Line::from(spans))
        })
        .collect();

    let list = List::new(items).highlight_style(
        Style::default()
            .bg(if focused {
                Color::Rgb(38, 48, 56)
            } else {
                Color::Reset
            })
            .add_modifier(Modifier::BOLD),
    );
    let mut state = ListState::default().with_selected(Some(app.sel_button));
    frame.render_stateful_widget(list, area, &mut state);
}

fn draw_props(frame: &mut Frame, app: &App, area: Rect, focused: bool) {
    let device = app.device();
    if device.props.is_empty() {
        empty(
            frame,
            area,
            "No editable libinput properties on this device.",
        );
        return;
    }

    let width = device
        .props
        .iter()
        .map(|p| p.prop.name.len())
        .max()
        .unwrap_or(20)
        .min(42);

    let items: Vec<ListItem> = device
        .props
        .iter()
        .map(|row| {
            let name = row.prop.name.trim_start_matches("libinput ").to_string();
            let mut spans = vec![
                Span::styled(
                    format!("{name:<width$}  "),
                    Style::default().fg(Color::Gray),
                ),
                Span::styled(
                    row.display(),
                    Style::default().fg(if row.is_dirty() {
                        PENDING
                    } else {
                        Color::Green
                    }),
                ),
            ];
            if row.is_dirty() {
                spans.push(Span::styled(" (staged)", Style::default().fg(PENDING)));
            }
            ListItem::new(Line::from(spans))
        })
        .collect();

    let list = List::new(items).highlight_style(
        Style::default()
            .bg(if focused {
                Color::Rgb(38, 48, 56)
            } else {
                Color::Reset
            })
            .add_modifier(Modifier::BOLD),
    );
    let mut state = ListState::default().with_selected(Some(app.sel_prop));
    frame.render_stateful_widget(list, area, &mut state);
}

fn draw_attrs(frame: &mut Frame, app: &App, area: Rect, focused: bool) {
    let device = app.device();
    let Some(node) = &device.sysfs else {
        empty(
            frame,
            area,
            "No sysfs node matched this device. Only PS/2 devices on the serio \
             bus expose kernel-side tuning; an I²C-HID pointer has none.",
        );
        return;
    };

    let columns = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(3), Constraint::Length(3)])
        .split(area);

    let items: Vec<ListItem> = node
        .attrs
        .iter()
        .map(|attr| {
            let range = match attr.kind {
                AttrKind::Range(lo, hi) => format!("{lo}–{hi}"),
                AttrKind::Bool => "0/1".to_string(),
            };
            let mut spans = vec![
                Span::styled(
                    format!("{:<16}", attr.name),
                    Style::default().fg(Color::Gray),
                ),
                Span::styled(
                    format!("{:>4}", attr.pending),
                    Style::default()
                        .fg(if attr.is_dirty() {
                            PENDING
                        } else {
                            Color::Green
                        })
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(format!("  [{range}]"), Style::default().fg(DIM)),
            ];
            if attr.is_dirty() {
                spans.push(Span::styled(
                    format!("  live: {}", attr.value),
                    Style::default().fg(PENDING),
                ));
            }
            if !attr.writable_directly {
                spans.push(Span::styled("  root", Style::default().fg(DIM)));
            }
            ListItem::new(Line::from(spans))
        })
        .collect();

    let list = List::new(items).highlight_style(
        Style::default()
            .bg(if focused {
                Color::Rgb(38, 48, 56)
            } else {
                Color::Reset
            })
            .add_modifier(Modifier::BOLD),
    );
    let mut state = ListState::default().with_selected(Some(app.sel_attr));
    frame.render_stateful_widget(list, columns[0], &mut state);

    let help = node.attrs.get(app.sel_attr).map(|a| a.help).unwrap_or("");
    frame.render_widget(
        Paragraph::new(help)
            .style(Style::default().fg(DIM))
            .wrap(Wrap { trim: true })
            .block(
                Block::default()
                    .borders(Borders::TOP)
                    .border_style(Style::default().fg(DIM)),
            ),
        columns[1],
    );
}

/// The equivalent shell command for whatever is selected — the thing that makes
/// the tool teachable rather than magic.
fn draw_hint(frame: &mut Frame, app: &App, area: Rect) {
    let device = app.device();
    let text = match app.tab {
        Tab::Buttons if !device.pending_buttons.is_empty() => {
            crate::xinput::button_map_command(&device.name, &device.pending_buttons)
        }
        Tab::Libinput => device
            .props
            .get(app.sel_prop)
            .map(|r| crate::xinput::set_prop_command(&device.name, &r.prop.name, &r.pending))
            .unwrap_or_default(),
        Tab::Sysfs => device
            .sysfs
            .as_ref()
            .and_then(|n| n.attrs.get(app.sel_attr))
            .map(|a| format!("echo {} | sudo tee {}", a.pending, a.path.display()))
            .unwrap_or_default(),
        _ => String::new(),
    };

    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            text,
            Style::default().fg(Color::Magenta),
        )))
        .wrap(Wrap { trim: true })
        .block(
            Block::default()
                .borders(Borders::TOP)
                .border_style(Style::default().fg(DIM))
                .title(Span::styled(
                    " equivalent command ",
                    Style::default().fg(DIM),
                )),
        ),
        area,
    );
}

fn draw_status(frame: &mut Frame, app: &App, area: Rect) {
    let colour = match app.status.level {
        Level::Info => Color::Gray,
        Level::Good => Color::Green,
        Level::Warn => PENDING,
        Level::Bad => OFF,
    };
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            format!(" {}", app.status.text),
            Style::default().fg(colour),
        ))),
        area,
    );
}

fn draw_keys(frame: &mut Frame, app: &App, area: Rect) {
    let keys = if app.modal.is_some() {
        "esc close"
    } else {
        "↑↓ move · ←→ adjust · tab section · space toggle · a apply · s save · d detect · ? help · q quit"
    };
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            format!(" {keys}"),
            Style::default().fg(DIM),
        ))),
        area,
    );
}

fn centered(area: Rect, percent_x: u16, percent_y: u16) -> Rect {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(area);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(vertical[1])[1]
}

fn draw_modal(frame: &mut Frame, app: &mut App) {
    let area = centered(frame.area(), 74, 74);
    frame.render_widget(Clear, area);

    // Drain the detector channel before anything takes an immutable borrow.
    if let Some(Modal::Detect(detector)) = app.modal.as_mut() {
        detector.poll();
    }

    let (title, body): (String, Vec<Line>) = match app.modal.as_ref() {
        Some(Modal::Help) => ("Keys".to_string(), help_lines()),
        Some(Modal::Detect(detector)) => {
            let mut lines = vec![
                Line::from(Span::styled(
                    "Click any pointer button. The device that actually sent it \
                     is the one whose button map you need to change.",
                    Style::default().fg(Color::Gray),
                )),
                Line::from(""),
            ];
            if detector.hits.is_empty() {
                lines.push(Line::from(Span::styled(
                    "waiting…",
                    Style::default().fg(DIM),
                )));
            }
            let hits = detector.hits.clone();
            for hit in hits.iter().rev() {
                lines.push(Line::from(vec![
                    Span::styled(
                        format!("button {:<3}", hit.button),
                        Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
                    ),
                    Span::raw("  "),
                    Span::raw(app.device_name_for_id(hit.device_id)),
                ]));
            }
            if let Some(msg) = &detector.ended {
                lines.push(Line::from(""));
                lines.push(Line::from(Span::styled(
                    msg.clone(),
                    Style::default().fg(OFF),
                )));
            }
            ("Which device sent that?".to_string(), lines)
        }
        Some(Modal::Persist {
            rule,
            profile,
            choice,
        }) => {
            let showing_rule = *choice == 0;
            let content = if showing_rule {
                rule.clone()
            } else {
                profile.clone()
            };
            let mut lines = vec![Line::from(vec![
                Span::styled(
                    if showing_rule {
                        "▸ udev rule "
                    } else {
                        "  udev rule "
                    },
                    Style::default().fg(if showing_rule { ACCENT } else { DIM }),
                ),
                Span::styled(
                    if showing_rule {
                        "  X profile"
                    } else {
                        "▸ X profile"
                    },
                    Style::default().fg(if showing_rule { DIM } else { ACCENT }),
                ),
                Span::styled("      ← → switch · enter write", Style::default().fg(DIM)),
            ])];
            lines.push(Line::from(Span::styled(
                if showing_rule {
                    format!("target: {}  (needs root)", crate::persist::UDEV_PATH)
                } else {
                    format!("target: {}", crate::persist::profile_path().display())
                },
                Style::default().fg(Color::Magenta),
            )));
            lines.push(Line::from(""));
            for line in content.lines() {
                let style = if line.trim_start().starts_with('#') {
                    Style::default().fg(DIM)
                } else {
                    Style::default().fg(Color::Green)
                };
                lines.push(Line::from(Span::styled(line.to_string(), style)));
            }
            ("Make it permanent".to_string(), lines)
        }
        Some(Modal::Edit { title, buffer, .. }) => {
            let lines = vec![
                Line::from(Span::styled(
                    "Type a value, enter to stage, esc to cancel.",
                    Style::default().fg(DIM),
                )),
                Line::from(""),
                Line::from(vec![
                    Span::styled("› ", Style::default().fg(ACCENT)),
                    Span::styled(
                        buffer.clone(),
                        Style::default()
                            .fg(Color::White)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled("▏", Style::default().fg(ACCENT)),
                ]),
            ];
            (title.clone(), lines)
        }
        None => return,
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(ACCENT))
        .title(Span::styled(
            format!(" {title} "),
            Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
        ))
        .title_alignment(Alignment::Left);

    frame.render_widget(
        Paragraph::new(body).block(block).wrap(Wrap { trim: false }),
        area,
    );
}

fn help_lines() -> Vec<Line<'static>> {
    let rows = [
        ("↑ ↓ / j k", "move within the focused pane"),
        ("← →", "switch pane, or adjust the selected value"),
        ("tab / shift-tab", "cycle Buttons · libinput · sysfs"),
        ("space", "toggle: disable a button, flip an on/off setting"),
        ("e", "type a value for the selected setting"),
        ("a", "apply everything staged in this section"),
        ("u", "reset the button map to how it was at start-up"),
        ("s", "save — udev rule for sysfs, profile for X settings"),
        ("d", "detect which device sends a button press"),
        ("r", "re-read everything from the system"),
        ("q / esc", "quit"),
    ];
    let mut lines = vec![Line::from(Span::styled(
        "Changes are staged first and applied with a, so nothing moves under \
         you while you are looking at it.",
        Style::default().fg(Color::Gray),
    ))];
    lines.push(Line::from(""));
    for (key, what) in rows {
        lines.push(Line::from(vec![
            Span::styled(
                format!("  {key:<16}"),
                Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
            ),
            Span::styled(what.to_string(), Style::default().fg(Color::Gray)),
        ]));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "Disabling button 2 stops middle-click paste while leaving TrackPoint \
         scrolling intact: libinput consumes the button for scrolling before \
         the X button map is applied, and scroll events travel as buttons 4–7.",
        Style::default().fg(DIM),
    )));
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{Device, PropRow, Status};
    use crate::sysfs::{Attr, AttrKind, Node};
    use crate::xinput::{Prop, PropValue, XDevice};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use std::path::PathBuf;

    fn attr(name: &str, value: &str, pending: &str, kind: AttrKind) -> Attr {
        Attr {
            name: name.to_string(),
            help: "Force needed to move the pointer.",
            kind,
            value: value.to_string(),
            original: value.to_string(),
            pending: pending.to_string(),
            writable_directly: false,
            path: PathBuf::from("/sys/devices/platform/i8042/serio1").join(name),
        }
    }

    fn trackpoint() -> Device {
        Device {
            x: Some(XDevice {
                id: 13,
                name: "TPPS/2 Elan TrackPoint".into(),
                kind: crate::xinput::Kind::SlavePointer,
            }),
            name: "TPPS/2 Elan TrackPoint".into(),
            buttons: vec![1, 2, 3, 4, 5, 6, 7],
            pending_buttons: vec![1, 0, 3, 4, 5, 6, 7],
            original_buttons: vec![1, 2, 3, 4, 5, 6, 7],
            props: vec![PropRow {
                prop: Prop {
                    name: "libinput Accel Speed".into(),
                    value: PropValue::Numbers(vec!["0.000000".into()]),
                    read_only: false,
                },
                pending: vec!["0.000000".into()],
                original: vec!["0.000000".into()],
            }],
            sysfs: Some(Node {
                path: PathBuf::from("/sys/devices/platform/i8042/serio1"),
                description: "i8042 AUX port".into(),
                firmware_id: "PNP: LEN0321".into(),
                attrs: vec![
                    attr("sensitivity", "128", "90", AttrKind::Range(0, 255)),
                    attr("press_to_select", "0", "0", AttrKind::Bool),
                ],
            }),
            note: None,
        }
    }

    fn app(tab: Tab) -> App {
        App {
            devices: vec![trackpoint()],
            sel_device: 0,
            sel_button: 1,
            sel_prop: 0,
            sel_attr: 0,
            focus: Focus::Detail,
            tab,
            modal: None,
            status: Status::default(),
            should_quit: false,
            x11: true,
        }
    }

    fn render(app: &mut App) -> String {
        let mut terminal = Terminal::new(TestBackend::new(110, 30)).unwrap();
        terminal.draw(|frame| draw(frame, app)).unwrap();
        let buffer = terminal.backend().buffer().clone();
        let mut text = String::new();
        for y in 0..buffer.area.height {
            for x in 0..buffer.area.width {
                text.push_str(buffer[(x, y)].symbol());
            }
            text.push('\n');
        }
        text
    }

    #[test]
    fn buttons_tab_shows_the_staged_disable_and_its_live_value() {
        let screen = render(&mut app(Tab::Buttons));
        assert!(screen.contains("TPPS/2 Elan TrackPoint"));
        assert!(screen.contains("disabled"));
        assert!(screen.contains("staged, live: 2"));
        assert!(screen.contains("middle"));
    }

    #[test]
    fn buttons_tab_shows_the_equivalent_xinput_command() {
        let screen = render(&mut app(Tab::Buttons));
        assert!(
            screen.contains("xinput set-button-map"),
            "the hint bar should teach the command:\n{screen}"
        );
    }

    #[test]
    fn sysfs_tab_shows_the_range_and_the_pending_value() {
        let screen = render(&mut app(Tab::Sysfs));
        assert!(screen.contains("sensitivity"));
        assert!(screen.contains("0–255"));
        assert!(screen.contains("live: 128"));
        assert!(screen.contains("sudo tee"));
    }

    #[test]
    fn libinput_tab_strips_the_redundant_prefix() {
        let screen = render(&mut app(Tab::Libinput));
        assert!(screen.contains("Accel Speed"));
        assert!(!screen.contains("libinput Accel Speed  0.000000"));
    }

    #[test]
    fn a_device_with_no_sysfs_node_explains_why() {
        let mut a = app(Tab::Sysfs);
        a.devices[0].sysfs = None;
        let screen = render(&mut a);
        assert!(screen.contains("No sysfs node matched"));
    }

    #[test]
    fn help_modal_renders_over_the_page() {
        let mut a = app(Tab::Buttons);
        a.modal = Some(Modal::Help);
        let screen = render(&mut a);
        assert!(screen.contains("Keys"));
        assert!(screen.contains("apply everything staged"));
    }

    #[test]
    fn persist_modal_shows_the_rule_and_its_target() {
        let mut a = app(Tab::Buttons);
        a.open_persist();
        let screen = render(&mut a);
        assert!(screen.contains("/etc/udev/rules.d/70-thinkpoint.rules"));
        assert!(screen.contains("ATTR{sensitivity}"));
    }

    #[test]
    fn a_narrow_terminal_still_renders() {
        let mut a = app(Tab::Sysfs);
        let mut terminal = Terminal::new(TestBackend::new(40, 12)).unwrap();
        terminal.draw(|frame| draw(frame, &mut a)).unwrap();
    }
}
