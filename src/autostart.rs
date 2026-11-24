//! Getting `--restore` to run at session start.
//!
//! The X-side settings — button maps, libinput properties, whether a device is
//! enabled — live only as long as the X server does. Writing them to a profile
//! is half the job; something has to replay it. udev cannot, because none of it
//! is kernel state, so it falls to the window manager's autostart.
//!
//! This handles i3 specifically. Guessing at every window manager's config
//! format would be worse than saying plainly that it is not handled.

use std::path::PathBuf;

use crate::error::{Error, Result};

/// What gets added. `exec_always` rather than `exec` so a config reload
/// reapplies it, which is handy while you are still changing settings.
pub const LINE: &str = "exec_always --no-startup-id thinkpoint --restore";

const HEADER: &str = "# Replay ThinkPoint's pointer settings (added by thinkpoint)";

/// Where i3 keeps its config, in the order i3 itself looks.
pub fn config_path() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let xdg = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| home.as_ref().map(|h| h.join(".config")));

    let candidates = [
        xdg.as_ref().map(|c| c.join("i3").join("config")),
        home.as_ref().map(|h| h.join(".i3").join("config")),
    ];
    candidates.into_iter().flatten().find(|p| p.is_file())
}

/// Is the replay already wired up? Matches any invocation, not just ours, so
/// a line the user wrote themselves counts and we do not add a second one.
pub fn installed(config: &str) -> bool {
    config.lines().any(|line| {
        let line = line.trim();
        !line.starts_with('#') && line.contains("thinkpoint") && line.contains("--restore")
    })
}

/// The config with the autostart line added. Idempotent: returns None when
/// there is nothing to do.
pub fn with_line(config: &str) -> Option<String> {
    if installed(config) {
        return None;
    }
    let mut out = config.trim_end().to_string();
    out.push_str("\n\n");
    out.push_str(HEADER);
    out.push('\n');
    out.push_str(LINE);
    out.push('\n');
    Some(out)
}

/// Whether the replay is wired up, as far as we can tell.
pub fn status() -> Status {
    let Some(path) = config_path() else {
        return Status::NoConfig;
    };
    match std::fs::read_to_string(&path) {
        Ok(text) if installed(&text) => Status::Installed(path),
        Ok(_) => Status::Missing(path),
        Err(_) => Status::NoConfig,
    }
}

#[derive(Debug, Clone)]
pub enum Status {
    Installed(PathBuf),
    Missing(PathBuf),
    /// No i3 config to edit. Another window manager, most likely.
    NoConfig,
}

/// Add the line, keeping a copy of what was there. This edits the user's
/// window manager config, so it owes them a way back.
pub fn install() -> Result<String> {
    let Some(path) = config_path() else {
        return Err(Error::new(format!(
            "no i3 config found — add this to your session's autostart yourself: {LINE}"
        )));
    };

    let config = std::fs::read_to_string(&path)
        .map_err(|e| Error::new(format!("{}: {e}", path.display())))?;

    let Some(updated) = with_line(&config) else {
        return Ok(format!("{} already replays the profile", path.display()));
    };

    let backup = path.with_extension("thinkpoint.bak");
    std::fs::write(&backup, &config)
        .map_err(|e| Error::new(format!("could not write {}: {e}", backup.display())))?;
    std::fs::write(&path, updated)
        .map_err(|e| Error::new(format!("could not write {}: {e}", path.display())))?;

    Ok(format!(
        "Added to {} (previous saved as {}) — reload i3 to pick it up",
        path.display(),
        backup.display()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spots_a_line_we_added() {
        let config = format!("bindsym $mod+Return exec alacritty\n{LINE}\n");
        assert!(installed(&config));
    }

    #[test]
    fn spots_a_line_the_user_wrote_themselves() {
        // A different spelling of the same intent must still count, or we
        // would append a duplicate.
        let config = "exec --no-startup-id /usr/bin/thinkpoint --restore\n";
        assert!(installed(config));
    }

    #[test]
    fn a_commented_line_does_not_count() {
        let config = format!("# {LINE}\n");
        assert!(!installed(&config));
    }

    #[test]
    fn an_unrelated_config_is_not_installed() {
        assert!(!installed("exec_always --no-startup-id picom\n"));
    }

    #[test]
    fn adding_is_idempotent() {
        let config = "exec_always --no-startup-id picom\n";
        let once = with_line(config).expect("should add");
        assert!(once.contains(LINE));
        assert!(with_line(&once).is_none(), "second call must be a no-op");
    }

    #[test]
    fn the_existing_config_is_kept_intact() {
        let config = "bindsym $mod+d exec rofi\nexec_always --no-startup-id picom\n";
        let out = with_line(config).unwrap();
        assert!(out.starts_with("bindsym $mod+d exec rofi\n"));
        assert!(out.contains("picom"));
    }

    #[test]
    fn the_line_is_explained_in_the_config() {
        let out = with_line("exec picom\n").unwrap();
        assert!(out.contains("added by thinkpoint"));
    }
}
