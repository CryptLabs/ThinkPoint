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
            format!("  {}", env!("CARGO_PKG_VERSION")),
            Style::default().fg(ACCENT),
        ),
        Span::styled(
            "  ·  buttons, libinput and TrackPoint tuning",
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
            let mut spans = vec![Span::styled(
                d.name.clone(),
                if d.enabled && !d.floating {
                    Style::default()
                } else {
                    Style::default().fg(DIM).add_modifier(Modifier::CROSSED_OUT)
                },
            )];
            if d.floating {
                spans.push(Span::styled(
                    "  detached",
                    Style::default().fg(PENDING).add_modifier(Modifier::BOLD),
                ));
            } else if !d.enabled {
                spans.push(Span::styled(
                    "  off",
                    Style::default().fg(OFF).add_modifier(Modifier::BOLD),
                ));
            }
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
    let device = app.device();
    let title = if device.floating {
        format!("{} — detached", device.name)
    } else if device.enabled {
        device.name.clone()
    } else {
        format!("{} — disabled", device.name)
    };
    let outer = panel(&title, focused);
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

    // With the device list focused, the action in reach is the on/off toggle,
    // so show that command rather than one from a tab you are not looking at.
    if app.focus == Focus::Devices && device.id().is_some() {
        let text = if device.floating {
            crate::xinput::reattach_command(&device.name)
        } else {
            crate::xinput::set_enabled_command(&device.name, !device.enabled)
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
                    .title(Span::styled(" what t would run ", Style::default().fg(DIM))),
            ),
            area,
        );
        return;
    }

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
        "↑↓ move · space toggle · a apply · b middle · t on/off · p drift · m meter · s save · i about · ? help · q quit"
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

    // Drain the event channels before anything takes an immutable borrow.
    match app.modal.as_mut() {
        Some(Modal::Detect(detector)) => detector.poll(),
        Some(Modal::Meter(meter)) => meter.poll(),
        _ => {}
    }

    let (title, body): (String, Vec<Line>) = match app.modal.as_ref() {
        Some(Modal::Help) => ("Keys".to_string(), help_lines()),
        Some(Modal::About(about)) => ("About".to_string(), about_lines(about)),
        Some(Modal::Meter(meter)) => ("Drift meter".to_string(), meter_lines(app, meter)),
        Some(Modal::Password {
            buffer,
            action,
            error,
        }) => {
            let mut lines = vec![
                Line::from(Span::styled(
                    format!("Root is needed to {}.", action.describe()),
                    Style::default().fg(Color::Gray),
                )),
                Line::from(""),
                Line::from(vec![
                    Span::styled("password  ", Style::default().fg(Color::Gray)),
                    Span::styled(
                        "•".repeat(buffer.chars().count()),
                        Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
                    ),
                    Span::styled("▏", Style::default().fg(ACCENT)),
                ]),
                Line::from(""),
                Line::from(Span::styled(
                    "enter to continue · esc to cancel",
                    Style::default().fg(DIM),
                )),
            ];
            if let Some(message) = error {
                lines.push(Line::from(""));
                lines.push(Line::from(Span::styled(
                    message.clone(),
                    Style::default().fg(OFF),
                )));
            }
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                "Your password goes to sudo on standard input — never into a \
                 command line, never to a file, and never anywhere this program \
                 keeps it. It is overwritten in memory as soon as sudo has it. \
                 sudo caches the authentication for a few minutes afterwards, so \
                 further writes in this session will not ask again.",
                Style::default().fg(DIM),
            )));
            ("Authentication".to_string(), lines)
        }
        Some(Modal::MiddleButton(choice)) => (
            "Middle button".to_string(),
            middle_button_lines(app, choice),
        ),
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

fn middle_button_lines<'a>(app: &App, choice: &crate::app::MiddleButton) -> Vec<Line<'a>> {
    let mut lines = vec![
        Line::from(Span::styled(
            format!("On {}", app.device().name),
            Style::default().fg(Color::Gray),
        )),
        Line::from(""),
    ];

    let row = |selected: bool, on: bool, supported: bool, label: &str, detail: &str| {
        let marker = if !supported {
            Span::styled("  —  ", Style::default().fg(DIM))
        } else if on {
            Span::styled("  ●  ", Style::default().fg(Color::Green))
        } else {
            Span::styled("  ○  ", Style::default().fg(OFF))
        };
        let name_style = if !supported {
            Style::default().fg(DIM)
        } else if selected {
            Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Gray)
        };
        Line::from(vec![
            Span::styled(
                if selected { "▸" } else { " " },
                Style::default().fg(ACCENT),
            ),
            marker,
            Span::styled(format!("{label:<26}"), name_style),
            Span::styled(detail.to_string(), Style::default().fg(DIM)),
        ])
    };

    lines.push(row(
        choice.cursor == 0,
        choice.paste,
        choice.paste_supported,
        "Paste on click",
        if choice.paste_supported {
            "button 2 reaches applications"
        } else {
            "no middle button on this device"
        },
    ));
    lines.push(row(
        choice.cursor == 1,
        choice.scroll,
        choice.scroll_supported,
        "Scroll while held",
        if choice.scroll_supported {
            "libinput button scrolling"
        } else {
            "no scroll method on this device"
        },
    ));

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "space toggles the highlighted row · enter applies both · esc cancels",
        Style::default().fg(DIM),
    )));
    lines.push(Line::from(""));

    // The two settings live in different places, and saying where makes the
    // combination below understandable rather than magical.
    let device = app.device();
    if choice.paste_supported {
        let restore = device.original_buttons.get(1).copied().unwrap_or(2);
        let mut map = device.pending_buttons.clone();
        if map.len() > 1 {
            map[1] = if choice.paste { restore } else { 0 };
        }
        lines.push(Line::from(Span::styled(
            crate::xinput::button_map_command(&device.name, &map),
            Style::default().fg(Color::Magenta),
        )));
    }
    if choice.scroll_supported
        && let Some(index) = device.scroll_prop()
    {
        let mut values = device.props[index].pending.clone();
        if values.len() > crate::app::SCROLL_BUTTON_INDEX {
            values[crate::app::SCROLL_BUTTON_INDEX] =
                if choice.scroll { "1" } else { "0" }.to_string();
        }
        lines.push(Line::from(Span::styled(
            crate::xinput::set_prop_command(&device.name, crate::app::SCROLL_METHOD, &values),
            Style::default().fg(Color::Magenta),
        )));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "Scroll off, paste on is the stock arrangement. Scroll on, paste off is          the usual reason for coming here: the stick still scrolls, but a middle          click no longer dumps the selection into whatever has focus. libinput          takes the button for scrolling before the X button map is consulted, so          the two settings do not fight.",
        Style::default().fg(DIM),
    )));
    lines
}

fn meter_lines<'a>(app: &App, meter: &crate::detect::Meter) -> Vec<Line<'a>> {
    let mut lines = vec![
        Line::from(Span::styled(
            "Hands off the machine. Movement reported with nothing touching the \
             device is real drift; a still reading means what you are seeing \
             comes from pointer acceleration rather than the hardware.",
            Style::default().fg(Color::Gray),
        )),
        Line::from(""),
    ];

    let readings = meter.readings();
    let elapsed = meter.started.elapsed().as_secs_f64();

    if elapsed < 1.0 {
        lines.push(Line::from(Span::styled(
            "settling…",
            Style::default().fg(DIM),
        )));
        return lines;
    }

    if readings.is_empty() {
        lines.push(Line::from(Span::styled(
            "No movement at all in the last few seconds.",
            Style::default().fg(Color::Green),
        )));
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "If the pointer still creeps on screen, the cause is above the \
             driver — check the accel profile on the libinput tab.",
            Style::default().fg(DIM),
        )));
        return lines;
    }

    lines.push(Line::from(Span::styled(
        format!(
            "{:<30}{:>9}{:>9}{:>9}{:>8}   verdict",
            "device", "x /sec", "y /sec", "total", "events"
        ),
        Style::default().fg(DIM),
    )));

    for reading in &readings {
        let colour = match reading.magnitude() {
            m if m < 0.5 => Color::Green,
            m if m < 5.0 => PENDING,
            _ => OFF,
        };
        let mut name = app.device_name_for_id(reading.device_id);
        if name.chars().count() > 28 {
            name = name.chars().take(27).collect::<String>() + "…";
        }
        lines.push(Line::from(vec![
            Span::styled(format!("{name:<30}"), Style::default().fg(Color::Gray)),
            Span::styled(
                format!(
                    "{:>9.1}{:>9.1}{:>9.1}{:>8}",
                    reading.dx_per_sec,
                    reading.dy_per_sec,
                    reading.magnitude(),
                    reading.events
                ),
                Style::default().fg(colour).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("   {}", reading.verdict()),
                Style::default().fg(colour),
            ),
        ]));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "Counts per second over a five-second window, straight from the \
         device's own valuators — before pointer acceleration, so the numbers \
         describe the hardware rather than what the cursor did.",
        Style::default().fg(DIM),
    )));

    if let Some(msg) = &meter.ended {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            msg.clone(),
            Style::default().fg(OFF),
        )));
    }
    lines
}

/// A ThinkPad keyboard with the one red key in the middle of it. The dot is the
/// whole point of the machine and of this program, so it gets to be red.
///
/// Rows are padded by measured width rather than by counting characters in the
/// source, so the frame closes cleanly whatever the glyphs turn out to be.
fn logo_lines() -> Vec<Line<'static>> {
    const INNER: usize = 24;
    const INDENT: &str = "    ";

    let key = Style::default().fg(Color::Rgb(90, 94, 100));
    let frame = Style::default().fg(DIM);
    let dot = Style::default()
        .fg(Color::Rgb(220, 60, 50))
        .add_modifier(Modifier::BOLD);

    let row = "▂▂ ▂▂ ▂▂ ▂▂ ▂▂ ▂▂ ▂▂";
    let dot_left = "▂▂ ▂▂ ▂▂ ";
    let dot_right = "  ▂▂ ▂▂ ▂▂";

    // Split the leftover space either side, so a row of odd width still sits
    // centred rather than drifting left.
    let pads = |used: usize| {
        let spare = INNER.saturating_sub(used);
        (" ".repeat(spare / 2), " ".repeat(spare - spare / 2))
    };

    let plain = |content: &str| {
        let (left, right) = pads(content.chars().count());
        Line::from(vec![
            Span::styled(format!("{INDENT}│{left}"), frame),
            Span::styled(content.to_string(), key),
            Span::styled(format!("{right}│"), frame),
        ])
    };

    let (dot_left_pad, dot_right_pad) =
        pads(dot_left.chars().count() + 1 + dot_right.chars().count());

    vec![
        Line::from(Span::styled(
            format!("{INDENT}╭{}╮", "─".repeat(INNER)),
            frame,
        )),
        plain(row),
        Line::from(vec![
            Span::styled(format!("{INDENT}│{dot_left_pad}"), frame),
            Span::styled(dot_left.to_string(), key),
            Span::styled("●".to_string(), dot),
            Span::styled(dot_right.to_string(), key),
            Span::styled(format!("{dot_right_pad}│"), frame),
        ]),
        plain(row),
        Line::from(Span::styled(
            format!("{INDENT}╰{}╯", "─".repeat(INNER)),
            frame,
        )),
    ]
}

fn about_lines(about: &crate::app::About) -> Vec<Line<'static>> {
    let label = Style::default().fg(DIM);
    let value = Style::default().fg(Color::Gray);

    let mut lines = logo_lines();
    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled("    ", label),
        Span::styled(
            "ThinkPoint",
            Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("  {}", env!("CARGO_PKG_VERSION")),
            Style::default().fg(Color::Green),
        ),
    ]));
    lines.push(Line::from(vec![
        Span::styled("    ", label),
        Span::styled(env!("CARGO_PKG_DESCRIPTION"), label),
    ]));
    lines.push(Line::from(""));

    let field = |name: &str, text: String| {
        Line::from(vec![
            Span::styled(format!("    {name:<12}"), label),
            Span::styled(text, value),
        ])
    };

    lines.push(field("author", env!("CARGO_PKG_AUTHORS").to_string()));
    lines.push(field("website", env!("CARGO_PKG_HOMEPAGE").to_string()));
    lines.push(field("source", env!("CARGO_PKG_REPOSITORY").to_string()));
    lines.push(field(
        "licence",
        format!("{} — see LICENSE", env!("CARGO_PKG_LICENSE")),
    ));
    lines.push(field("built with", "Rust and ratatui".to_string()));
    lines.push(Line::from(""));

    // The state of the session, which is the first thing anyone asks about in a
    // bug report and the last thing anyone thinks to look up.
    lines.push(Line::from(Span::styled("    this session", label)));
    let yes_no = |ok: bool, yes: &'static str, no: &'static str| {
        if ok {
            Span::styled(yes.to_string(), Style::default().fg(Color::Green))
        } else {
            Span::styled(no.to_string(), Style::default().fg(PENDING))
        }
    };
    lines.push(Line::from(vec![
        Span::styled("    xinput      ", label),
        yes_no(about.xinput, "available", "missing — sysfs only"),
    ]));
    lines.push(Line::from(vec![
        Span::styled("    devices     ", label),
        Span::styled(
            format!(
                "{} listed, {} with sysfs tuning",
                about.devices, about.with_sysfs
            ),
            value,
        ),
    ]));
    lines.push(Line::from(vec![
        Span::styled("    root        ", label),
        if about.root {
            Span::styled("running as root".to_string(), Style::default().fg(PENDING))
        } else {
            yes_no(
                about.sudo_cached,
                "sudo ready, no prompt needed",
                "sudo will ask for a password",
            )
        },
    ]));

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "    Bug reports and patches welcome at the source link above. Please \
         include the version line and what this screen says about your session.",
        label,
    )));
    lines
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
        ("t", "turn the device off or on, or reattach a detached one"),
        ("b", "middle button: choose paste and scroll independently"),
        ("p", "stage the drift-reducing preset on this device"),
        ("s", "save — udev rule for sysfs, profile for X settings"),
        ("d", "detect which device sends a button press"),
        ("m", "measure drift with your hands off the machine"),
        ("r", "re-read everything from the system"),
        ("i", "about: version, links and what this session found"),
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
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "On drift: the preset lowers sensitivity, which scales down the motion \
         a spurious force produces, and raises drift_time where the device has \
         one. Only IBM TrackPoints do. On an Elan, ALPS or NXP stick the \
         firmware handles drift correction and nothing here can change it — \
         that leaves the cap and a BIOS update.",
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
            floating: false,
            enabled: true,
            originally_enabled: true,
            sysfs: Some(Node {
                path: PathBuf::from("/sys/devices/platform/i8042/serio1"),
                description: "i8042 AUX port".into(),
                firmware_id: "PNP: LEN0321".into(),
                input_names: vec!["TPPS/2 Elan TrackPoint".into()],
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
            master_pointer: Some(2),
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
    fn help_lists_the_drift_keys() {
        let mut a = app(Tab::Buttons);
        a.modal = Some(Modal::Help);
        let screen = render(&mut a);
        assert!(screen.contains("drift-reducing preset"));
        assert!(screen.contains("measure drift"));
    }

    #[test]
    fn the_drift_preset_reports_what_it_could_not_tune() {
        // No drift_time in this fixture, matching an Elan TrackPoint.
        let mut a = app(Tab::Sysfs);
        a.apply_drift_preset();
        let screen = render(&mut a);
        assert!(
            screen.contains("no drift_time"),
            "the status bar should be honest about the limit:\n{screen}"
        );
    }

    #[test]
    fn a_disabled_device_is_marked_in_the_list_and_the_panel() {
        let mut a = app(Tab::Buttons);
        a.devices[0].enabled = false;
        let screen = render(&mut a);
        assert!(
            screen.contains("off"),
            "device list should flag it:\n{screen}"
        );
        assert!(screen.contains("disabled"));
    }

    #[test]
    fn the_device_pane_shows_what_the_toggle_would_run() {
        let mut a = app(Tab::Buttons);
        a.focus = Focus::Devices;
        let screen = render(&mut a);
        assert!(screen.contains("xinput disable"), "{screen}");

        a.devices[0].enabled = false;
        let screen = render(&mut a);
        assert!(screen.contains("xinput enable"), "{screen}");
    }

    #[test]
    fn help_lists_the_on_off_key() {
        let mut a = app(Tab::Buttons);
        a.modal = Some(Modal::Help);
        let screen = render(&mut a);
        assert!(screen.contains("turn the device off or on"));
    }

    #[test]
    fn the_password_prompt_masks_what_you_type() {
        let mut a = app(Tab::Sysfs);
        a.modal = Some(Modal::Password {
            buffer: "hunter2".to_string(),
            action: crate::app::RootAction::Sysfs,
            error: None,
        });
        let screen = render(&mut a);
        assert!(screen.contains("•••••••"), "should be masked:\n{screen}");
        assert!(
            !screen.contains("hunter2"),
            "the password must never be drawn"
        );
        assert!(screen.contains("standard input"));
    }

    #[test]
    fn a_rejected_password_says_so_on_the_retry() {
        let mut a = app(Tab::Sysfs);
        a.modal = Some(Modal::Password {
            buffer: String::new(),
            action: crate::app::RootAction::Sysfs,
            error: Some("that password was not accepted".into()),
        });
        let screen = render(&mut a);
        assert!(screen.contains("not accepted"));
    }

    #[test]
    fn the_middle_button_panel_shows_both_switches_and_both_commands() {
        let mut a = app(Tab::Buttons);
        a.modal = Some(Modal::MiddleButton(crate::app::MiddleButton {
            paste: false,
            scroll: true,
            cursor: 0,
            paste_supported: true,
            scroll_supported: false,
        }));
        let screen = render(&mut a);
        assert!(screen.contains("Paste on click"));
        assert!(screen.contains("Scroll while held"));
        assert!(screen.contains("xinput set-button-map"));
    }

    #[test]
    fn an_unsupported_middle_button_row_is_marked_rather_than_hidden() {
        let mut a = app(Tab::Buttons);
        a.modal = Some(Modal::MiddleButton(crate::app::MiddleButton {
            paste: true,
            scroll: false,
            cursor: 0,
            paste_supported: true,
            scroll_supported: false,
        }));
        let screen = render(&mut a);
        assert!(screen.contains("no scroll method on this device"));
    }

    #[test]
    fn a_detached_device_is_shown_as_detached_not_hidden() {
        // The failure this exists to prevent: a floating device dropping out of
        // the list entirely, so there is no way to reattach it from here.
        let mut a = app(Tab::Buttons);
        a.devices[0].floating = true;
        let screen = render(&mut a);
        assert!(screen.contains("detached"), "{screen}");
        assert!(screen.contains("TPPS/2 Elan TrackPoint"));
    }

    #[test]
    fn a_detached_device_offers_the_reattach_command() {
        let mut a = app(Tab::Buttons);
        a.devices[0].floating = true;
        a.focus = Focus::Devices;
        let screen = render(&mut a);
        assert!(screen.contains("xinput reattach"), "{screen}");
    }

    fn about_screen(root: bool, sudo: bool, xinput: bool) -> App {
        let mut a = app(Tab::Buttons);
        a.modal = Some(Modal::About(crate::app::About {
            devices: 3,
            with_sysfs: 2,
            xinput,
            root,
            sudo_cached: sudo,
        }));
        a
    }

    #[test]
    fn about_shows_the_version_name_and_links() {
        let screen = render(&mut about_screen(false, true, true));
        assert!(screen.contains("ThinkPoint"));
        assert!(screen.contains(env!("CARGO_PKG_VERSION")));
        assert!(screen.contains("Al1nuX"));
        assert!(screen.contains("cryptlabs.com"));
        assert!(screen.contains("github.com/CryptLabs/ThinkPoint"));
        assert!(screen.contains("MIT"));
        if std::env::var("DUMP").is_ok() {
            panic!("\n{screen}");
        }
    }

    #[test]
    fn about_draws_the_logo() {
        let screen = render(&mut about_screen(false, true, true));
        assert!(screen.contains('●'), "the TrackPoint dot:\n{screen}");
        assert!(screen.contains('╭') && screen.contains('╯'));
    }

    #[test]
    fn the_logo_frame_closes_straight() {
        // The dot is one column where a key is two, which is exactly the kind
        // of thing that leaves a ragged edge nobody notices until it ships.
        // Measure the rows themselves rather than the screen, where box-drawing
        // characters make byte offsets a poor proxy for columns.
        let widths: Vec<usize> = logo_lines()
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.chars().count())
                    .sum()
            })
            .collect();

        assert_eq!(widths.len(), 5, "logo should be five rows");
        assert!(
            widths.iter().all(|w| *w == widths[0]),
            "every row must be the same width, got {widths:?}"
        );
    }

    #[test]
    fn about_reports_what_this_session_found() {
        let screen = render(&mut about_screen(false, true, true));
        assert!(screen.contains("3 listed, 2 with sysfs tuning"));
        assert!(screen.contains("sudo ready"));
        assert!(screen.contains("available"));
    }

    #[test]
    fn about_warns_when_a_password_will_be_needed() {
        let screen = render(&mut about_screen(false, false, true));
        assert!(screen.contains("sudo will ask for a password"));
    }

    #[test]
    fn about_says_when_xinput_is_missing() {
        let screen = render(&mut about_screen(false, false, false));
        assert!(screen.contains("sysfs only"));
    }

    #[test]
    fn the_title_bar_carries_the_version() {
        let mut a = app(Tab::Buttons);
        let screen = render(&mut a);
        let first_line = screen.lines().next().unwrap();
        assert!(
            first_line.contains(env!("CARGO_PKG_VERSION")),
            "version belongs on the title bar: {first_line}"
        );
    }

    #[test]
    fn a_narrow_terminal_still_renders() {
        let mut a = app(Tab::Sysfs);
        let mut terminal = Terminal::new(TestBackend::new(40, 12)).unwrap();
        terminal.draw(|frame| draw(frame, &mut a)).unwrap();
    }
}
