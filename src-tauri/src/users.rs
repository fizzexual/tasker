//! People signed in to this PC (the "Users" tab).

use crate::error::{CmdError, CmdResult};
use serde::Serialize;
use windows::core::PWSTR;
use windows::Win32::Foundation::GetLastError;
use windows::Win32::System::RemoteDesktop::{
    ProcessIdToSessionId, WTSDisconnectSession, WTSDomainName, WTSEnumerateSessionsW, WTSFreeMemory, WTSLogoffSession,
    WTSQuerySessionInformationW, WTSUserName, WTS_INFO_CLASS, WTS_SESSION_INFOW,
};

#[derive(Debug, Serialize, Clone, PartialEq, Eq)]
pub struct UserSession {
    pub id: u32,
    pub user: String,
    pub domain: String,
    /// `active`, `disconnected` or `other`.
    pub state: &'static str,
    /// The session this app runs in.
    pub current: bool,
}

pub fn state_name(state: i32) -> &'static str {
    match state {
        0 => "active",
        4 => "disconnected",
        _ => "other",
    }
}

fn session_text(id: u32, class: WTS_INFO_CLASS) -> String {
    let mut text = PWSTR::null();
    let mut bytes = 0u32;
    unsafe {
        if WTSQuerySessionInformationW(None, id, class, &mut text, &mut bytes).is_err() || text.is_null() {
            return String::new();
        }
        let value = text.to_string().unwrap_or_default();
        WTSFreeMemory(text.0 as *mut _);
        value
    }
}

pub fn current_session() -> u32 {
    let mut id = 0u32;
    unsafe {
        let _ = ProcessIdToSessionId(std::process::id(), &mut id);
    }
    id
}

/// Sessions that belong to a person (the services session and empty consoles are left out).
pub fn list() -> Vec<UserSession> {
    let mut out = Vec::new();
    let mine = current_session();
    unsafe {
        let mut sessions: *mut WTS_SESSION_INFOW = std::ptr::null_mut();
        let mut count = 0u32;
        if WTSEnumerateSessionsW(None, 0, 1, &mut sessions, &mut count).is_err() || sessions.is_null() {
            return out;
        }
        for s in std::slice::from_raw_parts(sessions, count as usize) {
            let user = session_text(s.SessionId, WTSUserName);
            if user.is_empty() {
                continue;
            }
            out.push(UserSession {
                id: s.SessionId,
                user,
                domain: session_text(s.SessionId, WTSDomainName),
                state: state_name(s.State.0),
                current: s.SessionId == mine,
            });
        }
        WTSFreeMemory(sessions as *mut _);
    }
    out
}

fn known(id: u32) -> CmdResult<UserSession> {
    list().into_iter().find(|s| s.id == id).ok_or_else(|| CmdError::not_found("That user session"))
}

/// Disconnects a session: its programs keep running, the screen goes to the sign-in page.
pub fn disconnect(id: u32) -> CmdResult<()> {
    let session = known(id)?;
    let what = format!("Disconnecting {}", session.user);
    unsafe { WTSDisconnectSession(None, id, false).map_err(|_| CmdError::from_win32(GetLastError().0, &what)) }
}

/// Signs a user out: every program of theirs is closed.
pub fn sign_off(id: u32) -> CmdResult<()> {
    let session = known(id)?;
    let what = format!("Signing out {}", session.user);
    unsafe { WTSLogoffSession(None, id, false).map_err(|_| CmdError::from_win32(GetLastError().0, &what)) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_person_running_this_is_listed_as_current_and_active() {
        let sessions = list();
        let mine: Vec<_> = sessions.iter().filter(|s| s.current).collect();
        assert_eq!(mine.len(), 1, "{sessions:?}");
        assert_eq!(mine[0].state, "active");
        assert_eq!(mine[0].id, current_session());
        assert!(sessions.iter().all(|s| !s.user.is_empty()));
    }

    #[test]
    fn a_session_that_does_not_exist_is_refused_before_touching_anything() {
        assert_eq!(disconnect(987_654).unwrap_err().code, "not-found");
        assert_eq!(sign_off(987_654).unwrap_err().code, "not-found");
    }

    #[test]
    fn states_have_plain_names() {
        assert_eq!(state_name(0), "active");
        assert_eq!(state_name(4), "disconnected");
        assert_eq!(state_name(6), "other");
    }
}
