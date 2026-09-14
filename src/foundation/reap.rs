//! Ending everything this engine started, however far from it that has got.
//!
//! **The first line of defence is codex's own:** closed on its stdin, codex exits and ends
//! every process it started, including one a command detached (measured; see
//! `codex::process::close_codex`). An engine that dies any way at all closes that pipe too.
//! What is left for this module is what escapes codex, and what codex never started:
//!
//! - **A daemon that made a session of its own.** `adb` forks its server away and exits.
//!   Observed 2026-09-14: an `adb fork-server` from 2026-09-09, still running after every
//!   restart since, because nothing about codex reaches it.
//! - **A codex that is killed rather than closed** — one that would not exit on a closed
//!   stdin, or one that crashed.
//! - **Every other child of the engine** — a headless render, an `ffmpeg` — left behind by an
//!   engine that died without unwinding.
//!
//! None of those can be found by the process tree: a daemon belongs to init while its session
//! is live, and a crashed parent leaves nothing linking its children to anything. So
//! **ownership is written into every process, as environment it inherits:** [`mark_engine`]
//! stamps [`ENGINE_VAR`] (`<pid>.<start>`) into the engine's own environment at startup, so
//! every child of every subsystem carries it, and each codex spawn adds [`SESSION_VAR`]. Codex
//! passes the environment through to its commands whole — measured on the live processes:
//! `HI_AGENT_*` reach a `node` three hops below codex — and a fork or a detach keeps it.
//!
//! **A mark cannot always be read back.** macOS withholds the environment of its own
//! platform binaries (`/bin/sh`, `/bin/zsh`, `grep`, `tail`) from a non-root reader, and a
//! process that rewrites its title (`npm start`) overwrites it — measured: about half the
//! processes below the live engine answer with no environment at all. So a marked process
//! stands for more than itself. Selection starts from the processes whose mark answers and
//! grows to a fixpoint through everything below them and every process group they are in —
//! but only a group that is plainly ours, which keeps out the engine's own group and a shell's
//! the engine was started in ([`grow`] has the rule).
//!
//! Which marks count is the [`Scope`]:
//!
//! - a **session** closing ends what carries this engine *and* that session;
//! - a **clean stop** ends what carries this engine;
//! - a **boot** ends what carries an engine that is no longer running — the pid gone, or
//!   held by a process with a different start time. The pid alone would not do: pids are
//!   reused. A pid with its start time names one process for the life of the machine.
//!
//! All three are events. Nothing sweeps.
//!
//! **What this does not reach**, and it is not finished until something does:
//!
//! - **An escaped process none of whose relatives can be read** — on macOS, a daemon built
//!   only of platform binaries that left codex's group, after codex was killed rather than
//!   closed. Found by nothing.
//! - **A process that throws its environment away and leaves its group** — `env -i` plus a
//!   new session, `launchctl`. A system trigger is forbidden by `host.md` anyway.
//! - **A process running as another uid**, whose table entry is not even listed.
//! - **Windows.** There is no reader there, so nothing is marked and nothing is ended; codex's
//!   own close is all there is. The shell's job object should end everything with the engine
//!   (unverified — that shell has never run).

use std::collections::HashSet;
use std::sync::OnceLock;
use std::time::Duration;

/// The engine a process belongs to: `<pid>.<start>` of the engine that started it.
pub const ENGINE_VAR: &str = "HI_AGENT_ENGINE";
/// The codex session a process belongs to, within its engine.
pub const SESSION_VAR: &str = "HI_AGENT_SESSION";

/// How long a process gets to exit before it is made to. Enough for a shell to pass a signal
/// on, a script to flush a log line, codex to end its own commands; not a negotiation.
pub const GRACE: Duration = Duration::from_secs(2);

/// One process, named so the name cannot be reused: the pid and when it started.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Engine {
    pid: i32,
    start: u64,
}

impl Engine {
    fn render(self) -> String {
        format!("{}.{}", self.pid, self.start)
    }

    fn parse(s: &str) -> Option<Engine> {
        let (pid, start) = s.split_once('.')?;
        Some(Engine { pid: pid.parse().ok()?, start: start.parse().ok()? })
    }
}

/// This process's own name, or `None` where the table cannot be read — in which case
/// nothing is marked and nothing is ever ended.
pub fn this_engine() -> Option<Engine> {
    static THIS: OnceLock<Option<Engine>> = OnceLock::new();
    *THIS.get_or_init(|| {
        let pid = std::process::id() as i32;
        table().into_iter().find(|p| p.pid == pid).map(|p| Engine { pid, start: p.start })
    })
}

/// The value [`ENGINE_VAR`] carries for this process.
pub fn engine_mark() -> Option<String> {
    this_engine().map(Engine::render)
}

/// Stamp this process as an engine, so everything it ever starts carries the stamp.
///
/// Overwrites an inherited mark — an engine started by another engine's command is its own
/// engine for what *it* starts, while the outer one still owns the engine process itself,
/// whose exec-time environment is what the outer one reads. The session mark is removed for
/// the same reason: an engine is not inside a session of its own.
///
/// # Safety
///
/// Changes the process environment, so it must run before any thread exists — in `main`,
/// before the tray or any runtime is built.
pub unsafe fn mark_engine() {
    let Some(mark) = engine_mark() else { return };
    // SAFETY: the caller guarantees no other thread exists to read the environment.
    unsafe {
        std::env::set_var(ENGINE_VAR, mark);
        std::env::remove_var(SESSION_VAR);
    }
}

/// Which marks select a process.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    /// What this engine started for one codex session.
    Session(u64),
    /// Everything this engine started.
    Engine,
    /// Everything started by an engine that is no longer running.
    DeadEngines,
}

/// SIGTERM every process `scope` selects, and every process group it takes whole — the group
/// catches a fork that landed after the table was read. Returns what was signalled, for
/// [`Marked::kill_survivors`] after [`GRACE`].
pub fn terminate(scope: Scope) -> Marked {
    let marked = select(scope);
    marked.signal(Sig::Term);
    marked
}

/// End `scope` completely without waiting: SIGTERM now, SIGKILL for survivors after
/// [`GRACE`] on a plain thread. For a startup path or a closing session, where sleeping would
/// stall a runtime worker. A process about to exit may not live to send the SIGKILL, which is
/// why the clean stop awaits the grace itself.
pub fn end_detached(scope: Scope) -> usize {
    let marked = terminate(scope);
    let n = marked.len();
    if n > 0 {
        tracing::info!(?scope, processes = n, pids = ?marked.pids(), "ending what the engine started");
        std::thread::spawn(move || {
            std::thread::sleep(GRACE);
            marked.kill_survivors();
        });
    }
    n
}

/// What one [`terminate`] selected.
#[derive(Debug, Default)]
pub struct Marked {
    procs: Vec<Proc>,
    /// Groups taken whole, signalled as groups as well as member by member.
    groups: Vec<i32>,
}

impl Marked {
    pub fn len(&self) -> usize {
        self.procs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.procs.is_empty()
    }

    pub fn pids(&self) -> Vec<i32> {
        self.procs.iter().map(|p| p.pid).collect()
    }

    /// SIGKILL whatever is still there. Still there means the same pid with the same start
    /// time, so a pid reused in between by something unrelated is left alone.
    pub fn kill_survivors(&self) {
        let now: HashSet<(i32, u64)> = table().into_iter().map(|p| (p.pid, p.start)).collect();
        let procs: Vec<Proc> = self.procs.iter().copied().filter(|p| now.contains(&(p.pid, p.start))).collect();
        if procs.is_empty() {
            return;
        }
        let groups = self.groups.iter().copied().filter(|g| procs.iter().any(|p| p.pgid == *g)).collect();
        let survivors = Marked { procs, groups };
        tracing::warn!(pids = ?survivors.pids(), "processes outlived SIGTERM; killing");
        survivors.signal(Sig::Kill);
    }

    fn signal(&self, sig: Sig) {
        for p in &self.procs {
            send(p.pid, false, sig);
        }
        for &g in &self.groups {
            send(g, true, sig);
        }
    }
}

fn select(scope: Scope) -> Marked {
    let Some(me) = this_engine() else { return Marked::default() };
    let table = table();
    let mut env = EnvBuf::default();
    let mut seeds: HashSet<i32> = HashSet::new();
    let mut foreign: HashSet<i32> = HashSet::new();
    for p in table.iter().filter(|p| p.pid != me.pid) {
        match env.marks(p.pid) {
            Readable::Marked(m) if selected(scope, me, m, &table) => {
                seeds.insert(p.pid);
            }
            Readable::Marked(_) | Readable::Unmarked => {
                foreign.insert(p.pid);
            }
            Readable::No => {}
        }
    }
    grow(seeds, &foreign, &table, me.pid)
}

/// From the processes whose marks selected them, everything that goes with them: what runs
/// below them, and the members of every group they are in — to a fixpoint, since a group
/// member's children and a child's group both count. The engine itself is never taken.
///
/// A group is taken whole only when it is plainly ours. Its leader taken: yes. Its leader
/// alive and not taken: no — that is somebody else's group (the engine's own, a terminal's),
/// and only the selected members of it are. Its leader gone: only if no member is `foreign`,
/// a process whose environment answered and did not select it. That is what keeps a dead
/// engine that shared a shell's group from taking the shell's other jobs down with it.
fn grow(seeds: HashSet<i32>, foreign: &HashSet<i32>, table: &[Proc], engine: i32) -> Marked {
    let mut taken = seeds;
    let mut groups: HashSet<i32> = HashSet::new();
    loop {
        let before = (taken.len(), groups.len());
        for p in table {
            if p.pid != engine && !taken.contains(&p.pid) && taken.contains(&p.ppid) {
                taken.insert(p.pid);
            }
        }
        let candidate: HashSet<i32> =
            table.iter().filter(|p| taken.contains(&p.pid)).map(|p| p.pgid).collect();
        for g in candidate {
            let ours = if table.iter().any(|p| p.pid == g) {
                taken.contains(&g)
            } else {
                !table.iter().any(|p| p.pgid == g && foreign.contains(&p.pid) && !taken.contains(&p.pid))
            };
            if g > 1 && ours && groups.insert(g) {
                for p in table.iter().filter(|p| p.pgid == g && p.pid != engine) {
                    taken.insert(p.pid);
                }
            }
        }
        if (taken.len(), groups.len()) == before {
            break;
        }
    }
    let mut groups: Vec<i32> = groups.into_iter().collect();
    groups.sort_unstable();
    Marked { procs: table.iter().copied().filter(|p| taken.contains(&p.pid)).collect(), groups }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct Marks {
    engine: Option<Engine>,
    session: Option<u64>,
}

/// What reading one process's environment found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Readable {
    Marked(Marks),
    /// It answered, and carries no engine mark: not ours.
    Unmarked,
    /// It did not answer, or answered empty — which is what macOS gives for its own binaries.
    /// Could be anybody's.
    No,
}

fn selected(scope: Scope, me: Engine, marks: Marks, table: &[Proc]) -> bool {
    match scope {
        Scope::Session(id) => marks.engine == Some(me) && marks.session == Some(id),
        Scope::Engine => marks.engine == Some(me),
        Scope::DeadEngines => marks
            .engine
            .is_some_and(|e| e != me && !table.iter().any(|p| p.pid == e.pid && p.start == e.start)),
    }
}

/// The marks in one environment. An empty one reads as [`Readable::No`].
fn marks_in<'a>(env: impl IntoIterator<Item = &'a [u8]>) -> Readable {
    let mut marks = Marks::default();
    let mut any = false;
    for entry in env.into_iter().filter(|e| !e.is_empty()) {
        any = true;
        let Ok(entry) = std::str::from_utf8(entry) else { continue };
        let Some((key, value)) = entry.split_once('=') else { continue };
        match key {
            ENGINE_VAR => marks.engine = Engine::parse(value),
            SESSION_VAR => marks.session = value.parse().ok(),
            _ => {}
        }
    }
    match (any, marks.engine) {
        (false, _) => Readable::No,
        (true, None) => Readable::Unmarked,
        (true, Some(_)) => Readable::Marked(marks),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Proc {
    pid: i32,
    ppid: i32,
    pgid: i32,
    /// When the process started, in a platform unit that only has to be stable.
    start: u64,
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

/// Every process this uid owns. Nothing of another uid's could be signalled anyway.
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
        libc::proc_listallpids(pids.as_mut_ptr().cast(), (pids.len() * size_of::<libc::c_int>()) as libc::c_int)
    };
    pids.truncate(filled.max(0) as usize);

    // SAFETY: no preconditions.
    let uid = unsafe { libc::geteuid() };
    let size = size_of::<libc::proc_bsdinfo>() as libc::c_int;
    pids.into_iter()
        .filter(|&pid| pid > 0)
        .filter_map(|pid| {
            let mut info = MaybeUninit::<libc::proc_bsdinfo>::zeroed();
            // SAFETY: `info` is exactly `size` bytes; a short or failed read is rejected below.
            let n = unsafe { libc::proc_pidinfo(pid, libc::PROC_PIDTBSDINFO, 0, info.as_mut_ptr().cast(), size) };
            if n != size {
                return None;
            }
            // SAFETY: the call filled all `size` bytes.
            let info = unsafe { info.assume_init() };
            (info.pbi_uid == uid).then(|| Proc {
                pid,
                ppid: info.pbi_ppid as i32,
                pgid: info.pbi_pgid as i32,
                start: info.pbi_start_tvsec * 1_000_000 + info.pbi_start_tvusec,
            })
        })
        .collect()
}

#[cfg(target_os = "linux")]
fn table() -> Vec<Proc> {
    use std::os::unix::fs::MetadataExt;

    // SAFETY: no preconditions.
    let uid = unsafe { libc::geteuid() };
    let Ok(dir) = std::fs::read_dir("/proc") else {
        return Vec::new();
    };
    dir.filter_map(Result::ok)
        .filter(|entry| entry.metadata().is_ok_and(|m| m.uid() == uid))
        .filter_map(|entry| entry.file_name().to_str()?.parse::<i32>().ok())
        .filter_map(|pid| parse_stat(pid, &std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?))
        .collect()
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn table() -> Vec<Proc> {
    Vec::new()
}

/// `/proc/<pid>/stat` is `pid (comm) state ppid pgrp … starttime …`, and `comm` may itself
/// hold spaces and parentheses, so fields are counted from after the *last* `)`: `ppid` is
/// field 4, `pgrp` field 5, `starttime` field 22.
#[cfg(any(target_os = "linux", test))]
fn parse_stat(pid: i32, stat: &str) -> Option<Proc> {
    let fields: Vec<&str> = stat.get(stat.rfind(')')? + 1..)?.split_whitespace().collect();
    Some(Proc {
        pid,
        ppid: fields.get(1)?.parse().ok()?,
        pgid: fields.get(2)?.parse().ok()?,
        start: fields.get(19)?.parse().ok()?,
    })
}

/// Reads processes' environments, reusing one buffer across a whole table.
#[derive(Default)]
struct EnvBuf(Vec<u8>);

impl EnvBuf {
    #[cfg(target_os = "macos")]
    fn marks(&mut self, pid: i32) -> Readable {
        if self.0.is_empty() {
            let mut argmax: libc::c_int = 0;
            let mut len = std::mem::size_of::<libc::c_int>();
            let mut mib = [libc::CTL_KERN, libc::KERN_ARGMAX];
            // SAFETY: `argmax` is an int-sized out-parameter, as KERN_ARGMAX answers.
            let ok = unsafe {
                libc::sysctl(mib.as_mut_ptr(), 2, (&raw mut argmax).cast(), &mut len, std::ptr::null_mut(), 0)
            } == 0;
            self.0 = vec![0; if ok && argmax > 0 { argmax as usize } else { 1 << 20 }];
        }
        let mut mib = [libc::CTL_KERN, libc::KERN_PROCARGS2, pid];
        let mut len = self.0.len();
        // SAFETY: the kernel writes at most `len` bytes into the buffer and reports how many.
        let ok = unsafe {
            libc::sysctl(mib.as_mut_ptr(), 3, self.0.as_mut_ptr().cast(), &mut len, std::ptr::null_mut(), 0)
        } == 0;
        if !ok {
            return Readable::No;
        }
        marks_in(procargs_env(&self.0[..len.min(self.0.len())]))
    }

    #[cfg(target_os = "linux")]
    fn marks(&mut self, pid: i32) -> Readable {
        use std::io::Read;
        self.0.clear();
        let read = std::fs::File::open(format!("/proc/{pid}/environ")).and_then(|mut f| f.read_to_end(&mut self.0));
        if read.is_err() {
            return Readable::No;
        }
        marks_in(self.0.split(|&b| b == 0))
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    fn marks(&mut self, _pid: i32) -> Readable {
        Readable::No
    }
}

/// The environment entries in a `KERN_PROCARGS2` answer: an `int` argc, the exec path, NUL
/// padding, argc NUL-terminated arguments, then the environment up to the first empty entry
/// (the "apple" strings follow it and are not environment).
#[cfg(any(target_os = "macos", test))]
fn procargs_env(data: &[u8]) -> Vec<&[u8]> {
    let Some(argc) = data.get(..4).map(|b| i32::from_ne_bytes([b[0], b[1], b[2], b[3]]).max(0) as usize)
    else {
        return Vec::new();
    };
    let rest = &data[4..];
    let Some(path_end) = rest.iter().position(|&b| b == 0) else { return Vec::new() };
    let rest = &rest[path_end..];
    let rest = &rest[rest.iter().position(|&b| b != 0).unwrap_or(rest.len())..];
    let mut fields = rest.split(|&b| b == 0);
    for _ in 0..argc {
        if fields.next().is_none() {
            return Vec::new();
        }
    }
    fields.take_while(|f| !f.is_empty()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const ME: Engine = Engine { pid: 10, start: 5 };

    fn p(pid: i32, ppid: i32, pgid: i32) -> Proc {
        Proc { pid, ppid, pgid, start: 1 }
    }

    fn marks(engine: Option<Engine>, session: Option<u64>) -> Marks {
        Marks { engine, session }
    }

    fn taken(m: &Marked) -> Vec<i32> {
        let mut pids = m.pids();
        pids.sort_unstable();
        pids
    }

    #[test]
    fn a_session_takes_only_its_own_and_only_from_this_engine() {
        let table = [Proc { pid: 10, ppid: 1, pgid: 10, start: 5 }];
        let other = Engine { pid: 200, start: 7 };
        assert!(selected(Scope::Session(3), ME, marks(Some(ME), Some(3)), &table));
        assert!(!selected(Scope::Session(3), ME, marks(Some(ME), Some(4)), &table));
        assert!(!selected(Scope::Session(3), ME, marks(Some(ME), None), &table));
        // Session ids restart every boot; the engine mark is what keeps them apart.
        assert!(!selected(Scope::Session(3), ME, marks(Some(other), Some(3)), &table));
    }

    #[test]
    fn a_clean_stop_takes_everything_of_this_engine_and_nothing_else() {
        let table = [Proc { pid: 10, ppid: 1, pgid: 10, start: 5 }];
        assert!(selected(Scope::Engine, ME, marks(Some(ME), None), &table));
        assert!(selected(Scope::Engine, ME, marks(Some(ME), Some(9)), &table));
        assert!(!selected(Scope::Engine, ME, marks(Some(Engine { pid: 200, start: 7 }), None), &table));
    }

    #[test]
    fn a_boot_takes_what_a_dead_engine_left_and_leaves_a_live_one_alone() {
        let table = [Proc { pid: 10, ppid: 1, pgid: 10, start: 5 }, Proc { pid: 200, ppid: 1, pgid: 200, start: 7 }];
        let live = Engine { pid: 200, start: 7 };
        let dead = Engine { pid: 300, start: 8 };
        // Same pid as a live process, different start: the pid was reused, the engine is gone.
        let reused = Engine { pid: 200, start: 1 };
        assert!(!selected(Scope::DeadEngines, ME, marks(Some(ME), None), &table));
        assert!(!selected(Scope::DeadEngines, ME, marks(Some(live), Some(1)), &table));
        assert!(selected(Scope::DeadEngines, ME, marks(Some(dead), Some(1)), &table));
        assert!(selected(Scope::DeadEngines, ME, marks(Some(reused), None), &table));
    }

    /// The shapes observed below a live engine, where the readable marks are the minority.
    #[test]
    fn unreadable_processes_are_taken_through_their_relatives() {
        let table = [
            p(10, 1, 10),  // the engine, leading its own group
            p(20, 10, 10), // codex — readable, marked; in the engine's group
            p(30, 20, 30), // a command's zsh — unreadable, leads its group
            p(31, 30, 30), // grep under it — unreadable
            p(40, 1, 30),  // a `nohup tail &` left in that group after zsh exited — unreadable
            p(50, 1, 50),  // a daemon that made its own session — readable, marked
            p(51, 50, 50), // its unreadable child
            p(21, 10, 10), // another session's codex — not selected
            p(60, 21, 60), // its command
        ];
        // 21 and 60 answered with environments that did not select them.
        let foreign = HashSet::from([21, 60]);
        let marked = grow(HashSet::from([20, 50]), &foreign, &table, 10);
        assert_eq!(taken(&marked), vec![20, 30, 31, 40, 50, 51]);
        // The engine's group has a live leader outside the selection, so it is never signalled
        // as a group — that would reach the other session's codex and the engine itself.
        assert_eq!(marked.groups, vec![30, 50]);
    }

    #[test]
    fn a_leaderless_group_of_ours_is_taken_whole() {
        // A command's leader exited; one member is readable and marked, one cannot be read.
        let table = [p(10, 1, 10), p(41, 1, 30), p(42, 1, 30)];
        let marked = grow(HashSet::from([41]), &HashSet::new(), &table, 10);
        assert_eq!(taken(&marked), vec![41, 42]);
        assert_eq!(marked.groups, vec![30]);
    }

    #[test]
    fn a_leaderless_group_with_a_stranger_in_it_is_not_taken() {
        // A dead engine shared a shell's group (`nohup hi-agent &` over ssh), the shell is gone,
        // and another job the shell started is still in the group. It answered unmarked.
        let table = [p(10, 1, 10), p(70, 1, 5), p(71, 1, 5)];
        let marked = grow(HashSet::from([70]), &HashSet::from([71]), &table, 10);
        assert_eq!(taken(&marked), vec![70]);
        assert!(marked.groups.is_empty());
    }

    #[test]
    fn a_group_led_by_somebody_else_is_not_taken() {
        // The same, with the shell still alive and leading the group.
        let table = [p(5, 1, 5), p(70, 1, 5), p(71, 5, 5)];
        let marked = grow(HashSet::from([70]), &HashSet::new(), &table, 10);
        assert_eq!(taken(&marked), vec![70]);
        assert!(marked.groups.is_empty());
    }

    #[test]
    fn marks_are_read_out_of_an_environment() {
        let env: Vec<&[u8]> = vec![b"PATH=/bin", b"HI_AGENT_ENGINE=10.5", b"HI_AGENT_SESSION=3"];
        assert_eq!(marks_in(env), Readable::Marked(marks(Some(ME), Some(3))));
        let env: Vec<&[u8]> = vec![b"PATH=/bin", b"HI_AGENT_SESSION=3"];
        assert_eq!(marks_in(env), Readable::Unmarked);
        // What macOS answers for its own binaries.
        let env: Vec<&[u8]> = vec![];
        assert_eq!(marks_in(env), Readable::No);
    }

    #[test]
    fn procargs_yields_the_environment_after_the_arguments() {
        let mut data = 2i32.to_ne_bytes().to_vec();
        data.extend_from_slice(b"/bin/sh\0\0\0\0sh\0\0A=1\0B=2\0\0executable_path=/bin/sh\0");
        // argv is `sh` and an empty argument; the empty one must not end the arguments early.
        assert_eq!(procargs_env(&data), vec![&b"A=1"[..], &b"B=2"[..]]);
    }

    #[test]
    fn a_stat_line_is_read_after_the_last_paren() {
        let line = "4242 (a (weird) name) S 17 4240 4240 0 -1 4194560 1 0 0 0 0 0 0 0 20 0 1 0 987654 0";
        assert_eq!(parse_stat(4242, line), Some(Proc { pid: 4242, ppid: 17, pgid: 4240, start: 987654 }));
    }

    #[test]
    fn an_engine_mark_round_trips() {
        assert_eq!(Engine::parse(&ME.render()), Some(ME));
        assert_eq!(Engine::parse("10"), None);
    }

    /// Run by [`a_session_ends_what_it_started_however_detached`] as a child process, never on
    /// its own: without its variable it returns at once.
    #[test]
    #[ignore = "a helper process for the live reap test"]
    fn reap_helper() {
        use std::process::Command;
        match std::env::var("HI_AGENT_REAP_HELPER").as_deref() {
            // Leave a copy of ourselves behind in a group of its own and exit, so it is
            // reparented to init; that copy runs a platform binary whose environment macOS
            // will not show.
            Ok("detach") => {
                use std::os::unix::process::CommandExt;
                Command::new(std::env::current_exe().unwrap())
                    .args(["--exact", "foundation::reap::tests::reap_helper", "--ignored"])
                    .env("HI_AGENT_REAP_HELPER", "hold")
                    .process_group(0)
                    .spawn()
                    .unwrap();
            }
            Ok("hold") => {
                let _ = Command::new("sleep").arg("300").status();
            }
            _ => {}
        }
    }

    /// Against the real process table: a session's direct child, and a detached process in a
    /// group of its own running a child whose environment cannot be read. Ending the session
    /// ends all of them.
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn a_session_ends_what_it_started_however_detached() {
        use std::process::Command;
        use std::time::Instant;

        let me = this_engine().expect("the table names this process");
        // Unique to this run, so nothing else on the box can carry it.
        let session = 4_000_000_000 + u64::from(std::process::id());
        let helper = |mode: &str| {
            Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "foundation::reap::tests::reap_helper", "--ignored"])
                .env(ENGINE_VAR, me.render())
                .env(SESSION_VAR, session.to_string())
                .env("HI_AGENT_REAP_HELPER", mode)
                .stdout(std::process::Stdio::null())
                .spawn()
                .unwrap()
        };
        let mut attached = helper("hold");
        let _ = helper("detach").wait();

        // The attached helper, its sleep, the detached helper, its sleep.
        let deadline = Instant::now() + Duration::from_secs(10);
        while select(Scope::Session(session)).len() < 4 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(50));
        }
        let marked = terminate(Scope::Session(session));
        assert!(marked.len() >= 4, "expected both helpers and both sleeps, got {marked:?}");

        let deadline = Instant::now() + Duration::from_secs(5);
        let alive = |m: &Marked| {
            let now: HashSet<(i32, u64)> = table().into_iter().map(|p| (p.pid, p.start)).collect();
            m.procs.iter().filter(|p| now.contains(&(p.pid, p.start))).count()
        };
        while alive(&marked) > 0 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(50));
            let _ = attached.try_wait();
        }
        let _ = attached.wait();
        assert_eq!(alive(&marked), 0, "still running after terminate: {marked:?}");
    }
}
