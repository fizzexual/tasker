//! Pure process-tree logic: which processes belong to a tree, and which must never be ended.

use std::collections::{HashMap, HashSet};

/// The minimum we need to know about a process to build a tree safely.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcNode {
    pub pid: u32,
    pub ppid: u32,
    /// Seconds since the Unix epoch. Used to detect a reused parent PID.
    pub start_time: u64,
    pub name: String,
}

/// Processes Windows cannot live without. Ending one of them stops the PC with a blue screen,
/// so we refuse instead of asking.
const PROTECTED_NAMES: &[&str] = &[
    "system",
    "system idle process",
    "registry",
    "memory compression",
    "secure system",
    "smss.exe",
    "csrss.exe",
    "wininit.exe",
    "services.exe",
    "lsass.exe",
    "winlogon.exe",
];

pub fn is_protected(pid: u32, name: &str) -> bool {
    pid == 0 || pid == 4 || PROTECTED_NAMES.contains(&name.to_ascii_lowercase().as_str())
}

/// Returns `root` and all of its descendants, children before parents
/// (so ending them in this order never leaves an orphan that respawns a child).
///
/// Windows reuses PIDs and never clears a child's parent PID, so a process can name a
/// "parent" that is really an unrelated newer process. A real child is never older than its
/// parent, so such links are dropped.
pub fn collect_tree(procs: &[ProcNode], root: u32) -> Vec<u32> {
    let by_pid: HashMap<u32, &ProcNode> = procs.iter().map(|p| (p.pid, p)).collect();
    if !by_pid.contains_key(&root) {
        return Vec::new();
    }
    let mut children: HashMap<u32, Vec<u32>> = HashMap::new();
    for p in procs {
        if p.pid == p.ppid {
            continue;
        }
        if let Some(parent) = by_pid.get(&p.ppid) {
            if p.start_time >= parent.start_time {
                children.entry(p.ppid).or_default().push(p.pid);
            }
        }
    }

    let mut order = Vec::new();
    let mut seen = HashSet::new();
    let mut stack = vec![root];
    while let Some(pid) = stack.pop() {
        if !seen.insert(pid) {
            continue; // guards against a cycle in bad data
        }
        order.push(pid);
        if let Some(kids) = children.get(&pid) {
            stack.extend(kids.iter().copied());
        }
    }
    order.reverse();
    order
}

#[cfg(test)]
mod tests {
    use super::*;

    fn n(pid: u32, ppid: u32, start: u64) -> ProcNode {
        ProcNode { pid, ppid, start_time: start, name: format!("p{pid}.exe") }
    }

    #[test]
    fn collects_children_before_parents() {
        let procs = vec![n(10, 1, 100), n(11, 10, 101), n(12, 11, 102), n(20, 1, 100)];
        let tree = collect_tree(&procs, 10);
        assert_eq!(tree.len(), 3);
        assert_eq!(*tree.last().unwrap(), 10, "the root is ended last");
        let pos = |pid| tree.iter().position(|p| *p == pid).unwrap();
        assert!(pos(12) < pos(11));
        assert!(!tree.contains(&20), "a sibling is not part of the tree");
    }

    #[test]
    fn reused_parent_pid_is_not_a_child() {
        // 50 was started long ago by a parent with PID 10 that has since exited.
        // A new, unrelated process now owns PID 10.
        let procs = vec![n(10, 1, 500), n(50, 10, 100)];
        assert_eq!(collect_tree(&procs, 10), vec![10]);
    }

    #[test]
    fn cycle_in_bad_data_terminates() {
        let procs = vec![n(1, 2, 100), n(2, 1, 100)];
        let mut tree = collect_tree(&procs, 1);
        tree.sort();
        assert_eq!(tree, vec![1, 2]);
    }

    #[test]
    fn unknown_root_gives_nothing() {
        assert!(collect_tree(&[n(1, 0, 1)], 999).is_empty());
    }

    #[test]
    fn self_parent_is_ignored() {
        assert_eq!(collect_tree(&[n(0, 0, 0)], 0), vec![0]);
    }

    #[test]
    fn critical_windows_processes_are_protected() {
        assert!(is_protected(4, "System"));
        assert!(is_protected(0, "anything"));
        assert!(is_protected(812, "CSRSS.EXE"));
        assert!(is_protected(700, "lsass.exe"));
        assert!(!is_protected(4321, "chrome.exe"));
        assert!(!is_protected(4321, "explorer.exe"), "explorer can be restarted on purpose");
    }
}
