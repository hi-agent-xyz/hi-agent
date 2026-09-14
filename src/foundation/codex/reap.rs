//! Ending what a codex session's commands left running, before the session's codex ends.
//!
//! **Nothing done to codex reaches its commands.** Codex starts every command as the leader
//! of its own session — `ps` shows each shell it runs as `Ss`, in a process group of its own
//! — so `kill_on_drop`, which SIGKILLs codex, kills codex alone. The commands are reparented
//! to init and keep going, and from then on nothing links them to this process at all.
//!
//! Observed 2026-09-14: two headless Chrome trees, started by a view-reviewer's measurement
//! scripts that hung, were still spinning twelve cores three days after the graceful restart
//! (2026-09-11T03:34Z) that closed the session and SIGKILLed its codex. That boot was one of
//! seven that day, and each orphaned every command running at the time.
//!
//! `docs/arch/host.md` § *One background item* already says what an agent starts is a child
//! of this process tree, owned by the worker that owns the duty and dying when the engine
//! does. This is what makes that true: the tree below a codex is read by parent links **while
//! that codex is still alive** — the only moment the links exist — and terminated, and only
//! then is codex killed.
//!
//! **What this does not reach**, and it is not finished until something does:
//!
//! - **An engine that dies without unwinding** (SIGKILL, abort, power loss). Nothing here
//!   runs, codex exits on the closed pipe, and its commands are orphaned exactly as before.
//!   Carried in `host.md` § *Open*.
//! - **A codex that died on its own first.** Its commands were reparented the moment it
//!   exited, so the walk finds nothing below it.
//! - **Windows**, where there is no walk. The shell's job object should end everything with
//!   the engine (unverified — that shell has never run); a session closing mid-run is not
//!   covered there at all.

use std::collections::HashSet;
use std::time::Duration;

/// How long a command gets to exit on SIGTERM before it is killed outright. Enough for a
/// shell to pass the signal on and a script to flush a log line; not a negotiation.
pub(super) const GRACE: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Proc {
    pid: i32,
    ppid: i32,
    pgid: i32,
}

/// Everything below one process, as it stood when the table was read.
#[derive(Debug, Default)]
pub(super) struct Tree(Vec<Proc>);

impl Tree {
    /// Read the process table once and collect every descendant of `root`.
    pub(super) fn below(root: u32) -> Tree {
        Tree(descendants(root as i32, &table()))
    }

    pub(super) fn len(&self) -> usize {
        self.0.len()
    }

    pub(super) fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// SIGTERM every process in the tree, and every process group one of them leads.
    ///
    /// The groups are what catch a fork that landed after the table was read: codex's
    /// commands lead their own groups, so signalling the group reaches the whole command
    /// however it has grown since. A group is signalled only if its leader is in the tree,
    /// so this cannot reach the engine's own group, which no descendant of codex leads.
    pub(super) fn terminate(&self) {
        self.signal(Sig::Term);
    }

    /// SIGKILL whatever from the tree is still there. Still there means the same pid still
    /// carries the same group, so a pid reused in between by something unrelated is left alone.
    pub(super) fn kill_survivors(&self) {
        let now: HashSet<(i32, i32)> = table().into_iter().map(|p| (p.pid, p.pgid)).collect();
        let survivors: Vec<Proc> =
            self.0.iter().copied().filter(|p| now.contains(&(p.pid, p.pgid))).collect();
        if survivors.is_empty() {
            return;
        }
        tracing::warn!(
            pids = ?survivors.iter().map(|p| p.pid).collect::<Vec<_>>(),
            "command processes outlived SIGTERM; killing",
        );
        Tree(survivors).signal(Sig::Kill);
    }

    fn signal(&self, sig: Sig) {
        let pids: HashSet<i32> = self.0.iter().map(|p| p.pid).collect();
        for p in &self.0 {
            send(p.pid, false, sig);
        }
        let led: HashSet<i32> = self.0.iter().map(|p| p.pgid).filter(|g| pids.contains(g)).collect();
        for pgid in led {
            send(pgid, true, sig);
        }
    }
}

/// Every process whose parent chain reaches `root`, `root` itself excluded.
fn descendants(root: i32, table: &[Proc]) -> Vec<Proc> {
    let mut found: Vec<Proc> = Vec::new();
    let mut frontier: Vec<i32> = vec![root];
    let mut seen: HashSet<i32> = HashSet::from([root]);
    while let Some(parent) = frontier.pop() {
        for p in table.iter().filter(|p| p.ppid == parent) {
            // `seen` guards a table read mid-change, where a pid can turn up twice.
            if p.pid > 1 && seen.insert(p.pid) {
                found.push(*p);
                frontier.push(p.pid);
            }
        }
    }
    found
}

#[derive(Debug, Clone, Copy)]
enum Sig {
    Term,
    Kill,
}

#[cfg(unix)]
fn send(target: i32, group: bool, sig: Sig) {
    let sig = match sig {
        Sig::Term => libc::SIGTERM,
        Sig::Kill => libc::SIGKILL,
    };
    // SAFETY: plain syscalls. A target that already exited answers ESRCH, which is the goal.
    unsafe {
        if group {
            libc::killpg(target, sig);
        } else {
            libc::kill(target, sig);
        }
    }
}

#[cfg(not(unix))]
fn send(_target: i32, _group: bool, _sig: Sig) {}

#[cfg(target_os = "macos")]
fn table() -> Vec<Proc> {
    use std::mem::{MaybeUninit, size_of};

    // SAFETY: a null buffer asks only for the count.
    let count = unsafe { libc::proc_listallpids(std::ptr::null_mut(), 0) };
    if count <= 0 {
        return Vec::new();
    }
    // Headroom for processes started between the two calls.
    let mut pids: Vec<libc::c_int> = vec![0; count as usize + 64];
    // SAFETY: the size is given in bytes, as the call expects, and it writes at most that.
    let filled = unsafe {
        libc::proc_listallpids(
            pids.as_mut_ptr().cast(),
            (pids.len() * size_of::<libc::c_int>()) as libc::c_int,
        )
    };
    pids.truncate(filled.max(0) as usize);

    let size = size_of::<libc::proc_bsdinfo>() as libc::c_int;
    pids.into_iter()
        .filter(|&pid| pid > 0)
        .filter_map(|pid| {
            let mut info = MaybeUninit::<libc::proc_bsdinfo>::zeroed();
            // SAFETY: `info` is exactly `size` bytes; a short or failed read is rejected below.
            let n = unsafe {
                libc::proc_pidinfo(pid, libc::PROC_PIDTBSDINFO, 0, info.as_mut_ptr().cast(), size)
            };
            if n != size {
                return None;
            }
            // SAFETY: the call filled all `size` bytes.
            let info = unsafe { info.assume_init() };
            Some(Proc { pid, ppid: info.pbi_ppid as i32, pgid: info.pbi_pgid as i32 })
        })
        .collect()
}

#[cfg(target_os = "linux")]
fn table() -> Vec<Proc> {
    let Ok(dir) = std::fs::read_dir("/proc") else {
        return Vec::new();
    };
    dir.filter_map(Result::ok)
        .filter_map(|entry| entry.file_name().to_str()?.parse::<i32>().ok())
        .filter_map(|pid| parse_stat(pid, &std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?))
        .collect()
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn table() -> Vec<Proc> {
    Vec::new()
}

/// `/proc/<pid>/stat` is `pid (comm) state ppid pgrp …`, and `comm` may itself hold spaces
/// and parentheses, so the fields are read after the *last* `)`.
#[cfg(any(target_os = "linux", test))]
fn parse_stat(pid: i32, stat: &str) -> Option<Proc> {
    let mut rest = stat.get(stat.rfind(')')? + 1..)?.split_whitespace();
    let _state = rest.next()?;
    let ppid = rest.next()?.parse().ok()?;
    let pgid = rest.next()?.parse().ok()?;
    Some(Proc { pid, ppid, pgid })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(pid: i32, ppid: i32, pgid: i32) -> Proc {
        Proc { pid, ppid, pgid }
    }

    #[test]
    fn the_whole_subtree_is_found_and_nothing_beside_it() {
        let table = [
            p(10, 1, 10),  // the engine
            p(20, 10, 10), // codex
            p(30, 20, 30), // a command, leading its own group
            p(31, 30, 30), // what the command ran
            p(40, 31, 40), // a grandchild that made a group of its own
            p(21, 10, 10), // another codex — a sibling, not below
            p(50, 21, 50), // its command
        ];
        let mut pids: Vec<i32> = descendants(20, &table).iter().map(|p| p.pid).collect();
        pids.sort();
        assert_eq!(pids, vec![30, 31, 40]);
    }

    #[test]
    fn init_is_never_collected() {
        assert!(descendants(0, &[p(1, 0, 1)]).is_empty());
    }

    #[test]
    fn a_stat_line_is_read_after_the_last_paren() {
        let line = "4242 (a (weird) name) S 17 4242 4242 0 -1 4194560";
        assert_eq!(parse_stat(4242, line), Some(p(4242, 17, 4242)));
    }

    /// Against the real process table, in the shape codex produces: a stand-in parent runs
    /// a shell that leads its own process group, and that shell runs a child. Terminating
    /// the tree below the parent ends both, and leaves the parent alone.
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn a_command_leading_its_own_group_is_ended_from_its_parent() {
        use std::process::Command;
        use std::time::Instant;

        // `perl -e setpgrp` is the portable `setsid`-alike: macOS ships no `setsid` binary,
        // and perl is on both targets.
        let mut codex = Command::new("sh")
            .arg("-c")
            .arg(r#"perl -e 'setpgrp; exec "sh", "-c", "sleep 300 & wait"' & wait"#)
            .spawn()
            .expect("spawn the stand-in parent");
        let root = codex.id();

        let deadline = Instant::now() + Duration::from_secs(5);
        let tree = loop {
            let tree = Tree::below(root);
            let leads_a_group = tree.0.iter().any(|p| p.pid == p.pgid);
            if (tree.len() >= 2 && leads_a_group) || Instant::now() > deadline {
                break tree;
            }
            std::thread::sleep(Duration::from_millis(50));
        };
        assert!(
            tree.len() >= 2 && tree.0.iter().any(|p| p.pid == p.pgid),
            "expected a group-leading shell and its sleep below the parent, got {tree:?}",
        );

        tree.terminate();
        let deadline = Instant::now() + Duration::from_secs(5);
        let left = loop {
            let now: HashSet<i32> = table().into_iter().map(|p| p.pid).collect();
            let left: Vec<i32> = tree.0.iter().map(|p| p.pid).filter(|pid| now.contains(pid)).collect();
            if left.is_empty() || Instant::now() > deadline {
                break left;
            }
            std::thread::sleep(Duration::from_millis(50));
        };
        let _ = codex.wait();
        assert!(left.is_empty(), "still running after terminate: {left:?}");
    }
}
