//! One error shape for every command, so the UI can react to the *kind* of failure
//! (for example offer "Restart as administrator") instead of parsing text.

use serde::Serialize;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct CmdError {
    /// Stable machine-readable kind: `needs-admin`, `not-found`, `protected`,
    /// `cancelled`, `invalid`, `failed`.
    pub code: &'static str,
    /// Plain-language text shown to the user.
    pub message: String,
}

impl CmdError {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self { code, message: message.into() }
    }
    pub fn needs_admin(what: &str) -> Self {
        Self::new("needs-admin", format!("{what} needs administrator rights."))
    }
    pub fn not_found(what: &str) -> Self {
        Self::new("not-found", format!("{what} no longer exists."))
    }
    pub fn failed(message: impl Into<String>) -> Self {
        Self::new("failed", message)
    }
    pub fn invalid(message: impl Into<String>) -> Self {
        Self::new("invalid", message)
    }

    /// Map a raw Win32 error code to an error the UI understands.
    pub fn from_win32(code: u32, what: &str) -> Self {
        match code {
            5 => Self::needs_admin(what),
            87 | 1060 => Self::not_found(what),
            1051 => Self::failed(format!("{what}: other running services depend on it.")),
            1056 => Self::failed(format!("{what}: the service is already running.")),
            1058 => Self::failed(format!("{what}: the service is disabled.")),
            1062 => Self::failed(format!("{what}: the service is not running.")),
            1223 => Self::new("cancelled", "Cancelled."),
            other => Self::failed(format!("{what} failed (Windows error {other}).")),
        }
    }
}

pub type CmdResult<T> = Result<T, CmdError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn access_denied_is_needs_admin() {
        assert_eq!(CmdError::from_win32(5, "Ending the process").code, "needs-admin");
    }

    #[test]
    fn uac_cancel_is_cancelled_not_failure() {
        assert_eq!(CmdError::from_win32(1223, "x").code, "cancelled");
    }

    #[test]
    fn unknown_code_keeps_the_number_for_support() {
        let e = CmdError::from_win32(31, "Stopping the service");
        assert_eq!(e.code, "failed");
        assert!(e.message.contains("31"));
    }
}
