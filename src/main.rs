//! thinkpoint — a terminal front end for the pointer settings that usually
//! live in scattered sysfs files and half-remembered xinput incantations.

mod app;
mod detect;
mod error;
mod persist;
mod sysfs;
mod ui;
mod xinput;

use std::process::ExitCode;
use std::time::Duration;

use ratatui::crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use crate::app::{App, Focus, Level, Modal, Tab};
use crate::error::Result;

const VERSION: &str = env!("CARGO_PKG_VERSION");

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        None => run_tui(),
        Some("-h" | "--help") => {
            print_help();
            Ok(())
        }
        Some("-V" | "--version") => {
            println!("ThinkPoint {VERSION}");
            Ok(())
        }
        Some("--print-rule") => print_rule(),
        Some("--print-profile") => print_profile(),
        Some("--restore") => restore(),
        Some(other) => {
            eprintln!("thinkpoint: unknown argument {other:?}");
            print_help();
            return ExitCode::from(2);
        }
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("thinkpoint: {e}");
            ExitCode::FAILURE
        }
    }
}

fn print_help() {
    println!(
        "\
ThinkPoint {VERSION} — inspect and tune pointer devices

USAGE:
    thinkpoint                 open the terminal interface
    thinkpoint --restore       reapply the saved button maps and libinput
                               properties to the running X session
    thinkpoint --print-rule    print a udev rule for the current sysfs values
    thinkpoint --print-profile print the saved X profile
    thinkpoint --help          this text
    thinkpoint --version       version

WHAT IT TOUCHES:
    Button maps and libinput properties go through xinput and last until the
    session ends; --restore replays them, so it suits an autostart line.

    TrackPoint and psmouse knobs live in /sys and need root to write. The
    interface offers a udev rule so they survive a reboot and, unlike a
    session hook, a suspend/resume cycle too.

    Nothing is written without you asking. Values are staged, applied with 'a',
    and only saved to disk from the 's' screen."
    );
}

fn print_rule() -> Result<()> {
    let nodes = sysfs::scan_all();
    if nodes.is_empty() {
        eprintln!("thinkpoint: no serio devices with tunable attributes found");
    }
    print!("{}", persist::udev_rule(&nodes, false));
    Ok(())
}

fn print_profile() -> Result<()> {
    let settings = persist::load_profile()?;
    print!("{}", persist::render_profile(&settings));
    Ok(())
}

fn restore() -> Result<()> {
    let settings = persist::load_profile()?;
    if settings.is_empty() {
        println!("thinkpoint: profile is empty, nothing to restore");
        return Ok(());
    }

    let devices = xinput::list_pointers()?;
    let mut applied = 0usize;

    for setting in &settings {
        let Some(device) = devices.iter().find(|d| d.name == setting.device) else {
            eprintln!(
                "thinkpoint: {:?} is not connected, skipping",
                setting.device
            );
            continue;
        };
        if let Some(map) = &setting.button_map {
            xinput::set_button_map(device.id, map)?;
            applied += 1;
        }
        for (name, values) in &setting.props {
            xinput::set_prop(device.id, name, values)?;
            applied += 1;
        }
    }

    println!("thinkpoint: applied {applied} setting(s)");
    Ok(())
}

fn run_tui() -> Result<()> {
    let mut app = App::new()?;
    let mut terminal = ratatui::init();

    let outcome = loop {
        if let Err(e) = terminal.draw(|frame| ui::draw(frame, &mut app)) {
            break Err(e.into());
        }

        match event::poll(Duration::from_millis(120)) {
            Ok(true) => match event::read() {
                Ok(Event::Key(key)) if key.kind == KeyEventKind::Press => {
                    handle_key(&mut app, key);
                }
                Ok(_) => {}
                Err(e) => break Err(e.into()),
            },
            Ok(false) => {}
            Err(e) => break Err(e.into()),
        }

        if app.should_quit {
            break Ok(());
        }
    };

    ratatui::restore();
    outcome
}

fn handle_key(app: &mut App, key: KeyEvent) {
    if app.modal.is_some() {
        handle_modal_key(app, key);
        return;
    }

    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    match key.code {
        KeyCode::Char('q') | KeyCode::Esc => app.should_quit = true,
        KeyCode::Char('c') if ctrl => app.should_quit = true,

        KeyCode::Char('?') => app.modal = Some(Modal::Help),
        KeyCode::Char('d') => app.open_detector(),
        KeyCode::Char('s') => app.open_persist(),
        KeyCode::Char('r') => app.refresh(),
        KeyCode::Char('a') => app.apply(),
        KeyCode::Char('e') => app.open_editor(),
        KeyCode::Char('u') => app.reset_buttons(),

        KeyCode::Tab => {
            app.tab = app.tab.next();
            app.focus = Focus::Detail;
        }
        KeyCode::BackTab => {
            app.tab = app.tab.prev();
            app.focus = Focus::Detail;
        }

        KeyCode::Down | KeyCode::Char('j') => app.move_cursor(1),
        KeyCode::Up | KeyCode::Char('k') => app.move_cursor(-1),

        KeyCode::Left | KeyCode::Char('h') => match app.focus {
            Focus::Devices => {}
            Focus::Detail => match app.tab {
                Tab::Buttons => app.focus = Focus::Devices,
                Tab::Libinput => app.adjust_prop(-0.1),
                Tab::Sysfs => app.adjust_attr(-5),
            },
        },
        KeyCode::Right | KeyCode::Char('l') => match app.focus {
            Focus::Devices => app.focus = Focus::Detail,
            Focus::Detail => match app.tab {
                Tab::Buttons => {}
                Tab::Libinput => app.adjust_prop(0.1),
                Tab::Sysfs => app.adjust_attr(5),
            },
        },

        KeyCode::Enter => match app.focus {
            Focus::Devices => app.focus = Focus::Detail,
            Focus::Detail => app.apply(),
        },

        KeyCode::Char(' ') => match app.tab {
            Tab::Buttons => app.toggle_button(),
            Tab::Libinput => app.toggle_prop(),
            Tab::Sysfs => app.toggle_attr(),
        },

        _ => {}
    }
}

fn handle_modal_key(app: &mut App, key: KeyEvent) {
    // Text entry swallows almost everything, so it comes first.
    if let Some(Modal::Edit { buffer, target, .. }) = app.modal.as_mut() {
        match key.code {
            KeyCode::Esc => app.modal = None,
            KeyCode::Enter => {
                let (buffer, target) = (buffer.clone(), *target);
                app.modal = None;
                app.commit_editor(buffer, target);
            }
            KeyCode::Backspace => {
                buffer.pop();
            }
            KeyCode::Char(c) => buffer.push(c),
            _ => {}
        }
        return;
    }

    let is_persist = matches!(app.modal, Some(Modal::Persist { .. }));
    match key.code {
        KeyCode::Esc | KeyCode::Char('q') => app.modal = None,
        KeyCode::Left | KeyCode::Right | KeyCode::Tab if is_persist => {
            if let Some(Modal::Persist { choice, .. }) = app.modal.as_mut() {
                *choice = 1 - *choice;
            }
        }
        KeyCode::Enter if is_persist => {
            let payload = match app.modal.as_ref() {
                Some(Modal::Persist { rule, choice, .. }) => Some((rule.clone(), *choice)),
                _ => None,
            };
            app.modal = None;
            match payload {
                Some((rule, 0)) => {
                    if rule.contains("Nothing to persist") {
                        app.say(
                            "Nothing has changed, so there is nothing to write",
                            Level::Warn,
                        );
                    } else {
                        app.write_udev(&rule);
                    }
                }
                Some((_, _)) => app.write_profile(),
                None => {}
            }
        }
        _ => {}
    }
}
