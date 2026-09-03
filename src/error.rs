//! A deliberately small error type, so the crate stays dependency-light.

use std::fmt;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Clone)]
pub struct Error(pub String);

/// Sentinel message for "this needs root and we have no way in yet", so the
/// interface can answer with a prompt instead of an error.
pub const NEEDS_PASSWORD: &str = "root password required";

impl Error {
    pub fn new(msg: impl Into<String>) -> Self {
        Error(msg.into())
    }

    pub fn needs_password() -> Self {
        Error(NEEDS_PASSWORD.to_string())
    }

    pub fn is_needs_password(&self) -> bool {
        self.0 == NEEDS_PASSWORD
    }
}

/// Overwrite a string's bytes before dropping it. Not a guarantee against a
/// determined attacker with memory access — Rust may have moved the buffer on a
/// reallocation — but it keeps a password from sitting in the process image any
/// longer than it has to.
pub fn scrub(secret: &mut String) {
    // Safety: filling with zero bytes leaves valid UTF-8 (a run of NULs).
    unsafe { secret.as_mut_vec().fill(0) };
    secret.clear();
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error(e.to_string())
    }
}

impl From<std::num::ParseIntError> for Error {
    fn from(e: std::num::ParseIntError) -> Self {
        Error(e.to_string())
    }
}

/// Reject anything that would need quoting before it reaches a shell.
pub fn ensure_shell_safe(value: &str) -> Result<()> {
    if value.is_empty() {
        return Err(Error::new("value is empty"));
    }
    if value.len() > 64 {
        return Err(Error::new("value is implausibly long"));
    }
    if value
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | ' '))
    {
        Ok(())
    } else {
        Err(Error::new(format!("refusing to pass {value:?} to a shell")))
    }
}
