//! Ending processes and showing the facts about one.

use crate::error::{CmdError, CmdResult};
use crate::ntproc::{self, NtProc};
use crate::procinfo::{self, Handle};
use crate::proclist;
use crate::tree::{self, ProcNode};
use serde::Serialize;
use windows::core::BOOL;
use windows::Win32::Foundation::GetLastError;
use windows::Win32::System::Threading::{
    IsProcessCritical, OpenProcess, TerminateProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_TERMINATE,
};

/// A fresh list of every process, straight from Windows.
pub fn list() -> Vec<NtProc> {
    ntproc::query(&mut Vec::new())
}

/// A fresh list of every process, just enough to build trees and check names.
pub fn snapshot() -> CmdResult<Vec<ProcNode>> {
    let procs = list();
    if procs.is_empty() {
        return Err(CmdError::failed("Cannot read the process list."));
    }
    Ok(procs
        .into_iter()
        .map(|p| ProcNode { pid: p.pid, ppid: p.ppid, start_time: p.create_time, name: p.name })
        .collect())
}

#[derive(Debug, Serialize, Clone, PartialEq, Eq)]
pub struct KillFailure {
    pub pid: u32,
    pub name: String,
    pub code: &'static str,
    pub message: String,
}

#[derive(Debug, Serialize, Clone, PartialEq, Eq, Default)]
pub struct KillReport {
    pub ended: Vec<u32>,
    pub failed: Vec<KillFailure>,
}

/// Ends exactly one process. Refuses the ones Windows needs to stay alive.
fn terminate(pid: u32, name: &str) -> CmdResult<()> {
    let what = format!("Ending {name}");
    if tree::is_protected(pid, name) {
        return Err(CmdError::new("protected", format!("{name} is part of Windows. Ending it would stop the PC.")));
    }
    unsafe {
        let handle = OpenProcess(PROCESS_TERMINATE | PROCESS_QUERY_LIMITED_INFORMATION, false, pid)
            .map(Handle)
            .map_err(|_| CmdError::from_win32(GetLastError().0, &what))?;
        let mut critical = BOOL(0);
        if IsProcessCritical(handle.0, &mut critical).is_ok() && critical.as_bool() {
            return Err(CmdError::new(
                "protected",
                format!("{name} is marked critical by Windows. Ending it would stop the PC."),
            ));
        }
        TerminateProcess(handle.0, 1).map_err(|_| CmdError::from_win32(GetLastError().0, &what))
    }
}

/// Decides which processes an "end" request really covers. Pure, so it is tested without
/// ending anything.
///
/// - `expected_name` guards against a reused PID: if the process under that PID is no longer
///   the one the user clicked, nothing is ended.
/// - Our own process and everything under it (the window's WebView) is never part of a tree,
///   so ending the tree we were started from does not take the task manager down with it.
pub fn plan_kill(procs: &[ProcNode], pid: u32, expected_name: &str, whole_tree: bool, own_pid: u32) -> CmdResult<Vec<ProcNode>> {
    let target = procs.iter().find(|p| p.pid == pid).ok_or_else(|| CmdError::not_found("That process"))?;
    if !target.name.eq_ignore_ascii_case(expected_name) {
        return Err(CmdError::not_found("That process"));
    }
    if !whole_tree {
        return Ok(vec![target.clone()]);
    }
    let wanted = tree::collect_tree(procs, pid);
    // Only when the tree being ended is one we sit inside (not our own, not one of our children).
    let ours =
        if pid != own_pid && wanted.contains(&own_pid) { tree::collect_tree(procs, own_pid) } else { Vec::new() };
    Ok(wanted
        .into_iter()
        .filter(|p| !ours.contains(p))
        .filter_map(|p| procs.iter().find(|n| n.pid == p).cloned())
        .collect())
}

pub fn kill(pid: u32, expected_name: &str, whole_tree: bool) -> CmdResult<KillReport> {
    let procs = snapshot()?;
    let plan = plan_kill(&procs, pid, expected_name, whole_tree, std::process::id())?;

    let mut report = KillReport::default();
    for node in &plan {
        match terminate(node.pid, &node.name) {
            Ok(()) => report.ended.push(node.pid),
            // It ended by itself between the list and the request: that is the outcome wanted.
            Err(e) if e.code == "not-found" && node.pid != pid => report.ended.push(node.pid),
            Err(e) => report.failed.push(KillFailure {
                pid: node.pid,
                name: node.name.clone(),
                code: e.code,
                message: e.message,
            }),
        }
    }
    // A single process that could not be ended is a plain error, not a report to read.
    if !whole_tree {
        if let Some(f) = report.failed.first() {
            return Err(CmdError::new(f.code, f.message.clone()));
        }
    }
    Ok(report)
}

#[derive(Debug, Serialize, Clone, Default)]
pub struct ProcDetails {
    pub pid: u32,
    pub name: String,
    pub exe: String,
    pub command: String,
    pub user: String,
    pub description: String,
    pub company: String,
    pub parent_pid: u32,
    pub parent_name: String,
    /// Unix milliseconds.
    pub start_ms: u64,
    pub memory: u64,
    pub working_set: u64,
    pub peak_working_set: u64,
    pub commit: u64,
    pub page_faults: u32,
    pub cpu_time_ms: u64,
    pub threads: u32,
    pub handles: u32,
    pub session: u32,
    /// "normal", "high"… empty when Windows will not say.
    pub priority: &'static str,
    /// Processors this process may use / processors the PC has, as bit masks (as text: the
    /// screen cannot hold 64-bit whole numbers exactly).
    pub affinity: String,
    pub system_affinity: String,
    pub elevated: Option<bool>,
    pub architecture: &'static str,
    pub efficiency: bool,
    pub suspended: bool,
}

pub fn details(pid: u32) -> CmdResult<ProcDetails> {
    let procs = list();
    let p = procs.iter().find(|p| p.pid == pid).ok_or_else(|| CmdError::not_found("That process"))?;
    // A parent that started after its "child" is a reused PID, not the parent.
    let parent = procs.iter().find(|pp| pp.pid == p.ppid && pp.pid != pid && pp.create_time <= p.create_time);
    let exe = procinfo::exe_path(pid).unwrap_or_default();
    let (description, company) = procinfo::version_info(&exe);
    let (affinity, system_affinity) = procinfo::affinity_of(pid).unwrap_or((0, 0));
    Ok(ProcDetails {
        pid,
        name: p.name.clone(),
        command: procinfo::command_line(pid).unwrap_or_default(),
        user: procinfo::user_of(pid).unwrap_or_default(),
        description,
        company,
        parent_pid: parent.map(|pp| pp.pid).unwrap_or(0),
        parent_name: parent.map(|pp| pp.name.clone()).unwrap_or_default(),
        start_ms: p.start_unix_ms(),
        memory: p.private_ws,
        working_set: p.ws,
        peak_working_set: p.peak_ws,
        commit: p.commit,
        page_faults: p.page_faults,
        cpu_time_ms: p.cpu_time_ms(),
        threads: p.threads,
        handles: p.handles,
        session: p.session,
        priority: procinfo::priority_of(pid),
        affinity: affinity.to_string(),
        system_affinity: system_affinity.to_string(),
        elevated: procinfo::elevated(pid),
        architecture: procinfo::architecture(pid),
        efficiency: proclist::efficiency_of(pid).unwrap_or(false),
        suspended: p.suspended,
        exe,
    })
}

/// Facts that stay the same for a process's whole life, for the optional columns.
#[derive(Debug, Serialize, Clone, Default)]
pub struct StaticInfo {
    pub pid: u32,
    pub path: String,
    pub user: String,
    pub description: String,
    pub company: String,
    pub architecture: &'static str,
    pub elevated: Option<bool>,
    pub command: String,
}

pub fn static_info(pid: u32) -> StaticInfo {
    let path = procinfo::exe_path(pid).unwrap_or_default();
    let (description, company) = procinfo::version_info(&path);
    StaticInfo {
        pid,
        user: procinfo::user_of(pid).unwrap_or_default(),
        description,
        company,
        architecture: procinfo::architecture(pid),
        elevated: procinfo::elevated(pid),
        command: procinfo::command_line(pid).unwrap_or_default(),
        path,
    }
}

/// Path of a process's program file; `None` when Windows will not tell us (protected processes).
pub fn exe_path(pid: u32) -> Option<String> {
    procinfo::exe_path(pid)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::{Command, Stdio};

    fn n(pid: u32, ppid: u32, start: u64, name: &str) -> ProcNode {
        ProcNode { pid, ppid, start_time: start, name: name.into() }
    }

    #[test]
    fn single_kill_plans_only_the_target() {
        let procs = vec![n(10, 1, 5, "a.exe"), n(11, 10, 6, "b.exe")];
        let plan = plan_kill(&procs, 10, "a.exe", false, 999).unwrap();
        assert_eq!(plan.iter().map(|p| p.pid).collect::<Vec<_>>(), vec![10]);
    }

    #[test]
    fn tree_kill_plans_children_first() {
        let procs = vec![n(10, 1, 5, "a.exe"), n(11, 10, 6, "b.exe"), n(12, 11, 7, "c.exe")];
        let plan = plan_kill(&procs, 10, "A.EXE", true, 999).unwrap();
        assert_eq!(plan.iter().map(|p| p.pid).collect::<Vec<_>>(), vec![12, 11, 10]);
    }

    #[test]
    fn reused_pid_is_refused() {
        // The user clicked "chrome.exe" with PID 10; by the time the request arrives PID 10 is notepad.
        let procs = vec![n(10, 1, 5, "notepad.exe")];
        assert_eq!(plan_kill(&procs, 10, "chrome.exe", false, 999).unwrap_err().code, "not-found");
    }

    #[test]
    fn already_gone_is_not_found() {
        assert_eq!(plan_kill(&[], 10, "a.exe", true, 999).unwrap_err().code, "not-found");
    }

    #[test]
    fn tree_kill_never_includes_the_task_manager_itself() {
        // explorer(10) started us (50); we have a WebView child (51). Ending explorer's tree
        // must leave 50 and 51 alone but still end explorer's other child (60).
        let procs = vec![
            n(10, 1, 5, "explorer.exe"),
            n(50, 10, 6, "FizzTaskManager.exe"),
            n(51, 50, 7, "msedgewebview2.exe"),
            n(60, 10, 8, "other.exe"),
        ];
        let mut pids: Vec<u32> = plan_kill(&procs, 10, "explorer.exe", true, 50).unwrap().iter().map(|p| p.pid).collect();
        pids.sort();
        assert_eq!(pids, vec![10, 60]);
    }

    #[test]
    fn a_tree_under_the_task_manager_is_ended_in_full() {
        // We (50) started a helper (70) that started its own child (71). Ending 70's tree
        // is an explicit choice and must take 71 too.
        let procs = vec![n(50, 10, 6, "FizzTaskManager.exe"), n(70, 50, 7, "helper.exe"), n(71, 70, 8, "kid.exe")];
        let pids: Vec<u32> = plan_kill(&procs, 70, "helper.exe", true, 50).unwrap().iter().map(|p| p.pid).collect();
        assert_eq!(pids, vec![71, 70]);
    }

    #[test]
    fn ending_our_own_tree_ends_us_last() {
        let procs = vec![n(50, 10, 6, "FizzTaskManager.exe"), n(51, 50, 7, "msedgewebview2.exe")];
        let pids: Vec<u32> = plan_kill(&procs, 50, "FizzTaskManager.exe", true, 50).unwrap().iter().map(|p| p.pid).collect();
        assert_eq!(pids, vec![51, 50]);
    }

    #[test]
    fn protected_process_is_refused_before_touching_it() {
        let err = terminate(4, "System").unwrap_err();
        assert_eq!(err.code, "protected");
        let err = terminate(700, "csrss.exe").unwrap_err();
        assert_eq!(err.code, "protected");
    }

    #[test]
    fn ending_a_pid_that_does_not_exist_is_not_found() {
        // PIDs are multiples of 4 on Windows, so this one can never exist.
        let err = terminate(4_000_000_001, "ghost.exe").unwrap_err();
        assert_eq!(err.code, "not-found");
    }

    fn spawn_sleeper() -> std::process::Child {
        Command::new("cmd")
            .args(["/c", "ping -n 60 127.0.0.1 >nul"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("cmd.exe starts")
    }

    #[test]
    fn really_ends_a_process_we_started() {
        let mut child = spawn_sleeper();
        let report = kill(child.id(), "cmd.exe", false).unwrap();
        assert_eq!(report.ended, vec![child.id()]);
        let status = child.wait().unwrap();
        assert_eq!(status.code(), Some(1), "ended by us, not by finishing");
    }

    #[test]
    fn really_ends_a_whole_tree() {
        // cmd.exe starts ping.exe as its child; ending the tree must end both.
        let mut child = spawn_sleeper();
        std::thread::sleep(std::time::Duration::from_millis(700));
        let before = snapshot().unwrap();
        let kids: Vec<u32> = before.iter().filter(|p| p.ppid == child.id()).map(|p| p.pid).collect();
        assert!(!kids.is_empty(), "the child process is running");

        let report = kill(child.id(), "cmd.exe", true).unwrap();
        assert!(report.failed.is_empty(), "{:?}", report.failed);
        assert!(report.ended.contains(&child.id()));
        child.wait().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(300));
        let after = snapshot().unwrap();
        for kid in kids {
            assert!(!after.iter().any(|p| p.pid == kid && p.ppid == child.id()), "child {kid} is gone");
        }
    }

    #[test]
    fn wrong_name_ends_nothing() {
        let mut child = spawn_sleeper();
        let err = kill(child.id(), "definitely-not-cmd.exe", false).unwrap_err();
        assert_eq!(err.code, "not-found");
        assert!(child.try_wait().unwrap().is_none(), "the process is still running");
        child.kill().unwrap();
        child.wait().unwrap();
    }

    #[test]
    fn details_of_ourselves() {
        let d = details(std::process::id()).unwrap();
        assert!(d.exe.to_lowercase().ends_with(".exe"));
        assert!(d.memory > 0 && d.working_set >= d.memory);
        assert!(d.start_ms > 1_600_000_000_000);
        assert_eq!(exe_path(std::process::id()).unwrap(), d.exe);
        assert_eq!(d.priority, "normal");
        assert_eq!(d.architecture, "64-bit");
        assert!(!d.user.is_empty() && d.threads >= 1 && !d.suspended && !d.efficiency);
        assert!(d.affinity.parse::<u64>().unwrap() > 0);
        assert!(!d.parent_name.is_empty(), "cargo started us");
        let s = static_info(std::process::id());
        assert_eq!((s.path, s.user), (d.exe, d.user));
        assert!(static_info(4_000_000_001).path.is_empty(), "a missing process gives empty facts, not an error");
    }

    #[test]
    fn details_of_a_missing_process_is_not_found() {
        assert_eq!(details(4_000_000_001).unwrap_err().code, "not-found");
    }
}
