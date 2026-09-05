//! The engine, as a child process — or as something already running that this
//! shell attaches to instead.
//!
//! "Host and client are capabilities of an app instance, never properties of a
//! platform" (`docs/arch/topology.md`), and a desktop answers yes to hosting.
//! So the shell starts `hi-agent`, keeps it running, and — through
//! `PR_SET_PDEATHSIG` — makes sure it never outlives the shell.
//!
//! Supervision, not management. The engine owns its data directory, its runtime
//! provisioning and its own restarts of anything below it; all this does is
//! start it, watch for it going away, and start it again.

use std::cell::RefCell;
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::rc::Rc;
use std::time::{Duration, Instant};

use crate::paths::{engine_bin, engine_data, engine_log, log};

use super::client;
use super::models::HealthState;

/// The port a desktop install is reached on, per `src/main.rs`.
pub const PREFERRED_PORT: u16 = 12358;

#[derive(Default)]
struct EngineState {
    /// Where the engine is, once an address is known. `None` until then.
    base_url: Option<String>,
    /// The child, when this shell is the one that started it.
    pid: Option<i32>,
    /// True when this shell did not start the engine because one was already
    /// answering on the preferred port — a `systemd --user` unit, a developer
    /// running `make dev`, or a shell that crashed and left its child behind.
    /// Adopting is right: two engines over one data directory is the failure
    /// worth avoiding, and it is worse than not being the one who started it.
    adopted: bool,
    /// Last thing that went wrong, for the stage to show.
    failure: Option<String>,
    stopping: bool,
    backoff: Duration,
}

pub struct LocalCore {
    state: RefCell<EngineState>,
    observers: RefCell<Vec<Box<dyn Fn()>>>,
}

impl LocalCore {
    pub fn new() -> Rc<Self> {
        Rc::new(Self {
            state: RefCell::new(EngineState {
                backoff: Duration::from_secs(1),
                ..EngineState::default()
            }),
            observers: RefCell::new(Vec::new()),
        })
    }

    /// Raised when the engine's reachability changes.
    pub fn connect_changed(&self, observer: impl Fn() + 'static) {
        self.observers.borrow_mut().push(Box::new(observer));
    }

    pub fn base_url(&self) -> Option<String> {
        self.state.borrow().base_url.clone()
    }

    pub fn failure(&self) -> Option<String> {
        self.state.borrow().failure.clone()
    }

    /// Start the engine, or adopt one already running, and keep it up until
    /// [`Self::shutdown`]. Returns as soon as an address is known — health is
    /// polled by the caller, because a core that is up and one that answers are
    /// different facts and the second is the one the face needs.
    pub async fn start(self: &Rc<Self>) -> Option<String> {
        if let Some(base_url) = self.base_url() {
            return Some(base_url);
        }

        let existing = format!("http://127.0.0.1:{PREFERRED_PORT}");
        if client::health(&existing).await == HealthState::Here {
            {
                let mut state = self.state.borrow_mut();
                state.adopted = true;
                state.base_url = Some(existing.clone());
            }
            log(format!("adopted an engine already answering on {PREFERRED_PORT}"));
            self.notify();
            return Some(existing);
        }

        let Some(bin) = engine_bin() else {
            self.fail(
                "hi-agent is not installed beside this app. Reinstall, or add a core that runs elsewhere.",
            );
            return None;
        };

        let port = if port_is_free(PREFERRED_PORT) {
            PREFERRED_PORT
        } else {
            free_port()?
        };
        self.state.borrow_mut().base_url = Some(format!("http://127.0.0.1:{port}"));
        self.spawn(bin, port);
        self.base_url()
    }

    /// Start the engine and arrange to be told when it stops.
    ///
    /// The restart backoff exists for the case where the engine exits
    /// immediately and forever — a corrupt data directory, a missing
    /// dependency — where restarting in a tight loop would burn the machine and
    /// bury the reason in a log nobody can read.
    fn spawn(self: &Rc<Self>, bin: PathBuf, port: u16) {
        let data_dir = engine_data();
        log(format!(
            "starting {} --port {port} --data-dir {}",
            bin.display(),
            data_dir.display()
        ));

        let mut command = Command::new(&bin);
        command
            .arg("--port")
            .arg(port.to_string())
            .arg("--data-dir")
            .arg(&data_dir)
            // The engine resolves relative paths — including its `./data`
            // fallback — against this. The person's home is the least
            // surprising answer for a `.desktop` launch, whose working
            // directory is otherwise whatever the launcher happened to have,
            // and `--data-dir` means the fallback is never reached anyway.
            .current_dir(glib::home_dir())
            .stdin(Stdio::null());

        match std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(engine_log())
        {
            Ok(file) => match file.try_clone() {
                Ok(second) => {
                    command.stdout(Stdio::from(file)).stderr(Stdio::from(second));
                }
                Err(_) => {
                    command.stdout(Stdio::from(file)).stderr(Stdio::null());
                }
            },
            Err(e) => {
                log(format!("engine log not opened, output is dropped: {e}"));
            }
        }

        let parent = std::process::id() as libc::pid_t;
        // SAFETY: the closure runs between fork and exec, where only
        // async-signal-safe calls are allowed. `prctl`, `getppid` and `_exit`
        // all are.
        unsafe {
            command.pre_exec(move || {
                // What stops the engine outliving the shell. Windows needed a
                // job object because it has no orphan reaping; Linux has the
                // signal built in, so a child whose shell dies — crash, kill,
                // or a clean quit — gets SIGTERM without the shell having to
                // survive long enough to send it. The engine is built to be
                // killed: its state is on disk and its session directory is
                // written as it goes, which is what makes resuming after a
                // restart work at all.
                //
                // The signal is tied to the *thread* that forked, which is the
                // GTK main thread — it lives exactly as long as the process.
                if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
                // The window between fork and prctl: if the shell died inside
                // it the signal has already been missed, and this child would
                // be the orphan the whole mechanism exists to prevent.
                if libc::getppid() != parent {
                    libc::_exit(0);
                }
                Ok(())
            });
        }

        let child = match command.spawn() {
            Ok(child) => child,
            Err(e) => {
                log(format!("engine could not be started: {e}"));
                self.fail(&format!("The agent could not be started: {e}"));
                return;
            }
        };

        // Only the pid is kept. GLib's child watch is what reaps this process,
        // so nothing here may call `wait` — and `std::process::Child` has no
        // `Drop` that would, which is what makes handing ownership over safe.
        let pid = child.id() as i32;
        drop(child);
        {
            let mut state = self.state.borrow_mut();
            state.pid = Some(pid);
            state.failure = None;
        }
        self.notify();

        let started_at = Instant::now();
        let weak = Rc::downgrade(self);
        // `child_watch_add_local` takes an `FnMut` even though GLib fires it
        // once, so the path is cloned rather than moved out.
        glib::child_watch_add_local(glib::Pid(pid), move |_, status| {
            let Some(this) = weak.upgrade() else {
                return;
            };
            this.on_exit(bin.clone(), port, started_at, status);
        });
    }

    fn on_exit(self: &Rc<Self>, bin: PathBuf, port: u16, started_at: Instant, status: i32) {
        {
            let mut state = self.state.borrow_mut();
            state.pid = None;
            if state.stopping {
                return;
            }
            state.failure = Some(format!("The agent stopped (status {status}). Restarting."));
            // A run that lasted a while was working; the next failure is a new
            // one and deserves a fresh short wait rather than the last run's
            // accumulated punishment.
            state.backoff = if started_at.elapsed() > Duration::from_secs(60) {
                Duration::from_secs(1)
            } else {
                state.backoff.saturating_mul(2).min(Duration::from_secs(30))
            };
        }
        log(format!("engine exited with status {status}"));
        self.notify();

        let backoff = self.state.borrow().backoff;
        let weak = Rc::downgrade(self);
        glib::timeout_add_local_once(backoff, move || {
            let Some(this) = weak.upgrade() else {
                return;
            };
            if this.state.borrow().stopping {
                return;
            }
            this.spawn(bin, port);
        });
    }

    /// Stop an engine this shell started, and leave an adopted one alone.
    ///
    /// The one thing that must not be got wrong. A `systemd --user` unit is the
    /// answer to stock GNOME having no tray — it keeps the agent alive with no
    /// window open — and killing the engine it manages every time a window
    /// closes would make the unit pointless.
    pub fn shutdown(&self) {
        let pid = {
            let mut state = self.state.borrow_mut();
            state.stopping = true;
            if state.adopted { None } else { state.pid.take() }
        };
        let Some(pid) = pid else {
            return;
        };
        log(format!("stopping the engine ({pid})"));
        // SAFETY: `pid` is this process's own child and has not been reaped —
        // the child watch clears it before GLib waits on it.
        unsafe {
            libc::kill(pid, libc::SIGTERM);
        }
    }

    fn fail(&self, message: &str) {
        log(message);
        self.state.borrow_mut().failure = Some(message.to_string());
        self.notify();
    }

    fn notify(&self) {
        for observer in self.observers.borrow().iter() {
            observer();
        }
    }
}

/// Whether the engine's preferred port can be bound right now.
fn port_is_free(port: u16) -> bool {
    std::net::TcpListener::bind(("127.0.0.1", port)).is_ok()
}

/// A port the OS says is free. Racy by nature — something can take it between
/// the close and the engine's bind — and the supervisor's restart is what covers
/// that, rather than a lock that cannot exist.
fn free_port() -> Option<u16> {
    std::net::TcpListener::bind(("127.0.0.1", 0))
        .ok()
        .and_then(|listener| listener.local_addr().ok())
        .map(|address| address.port())
}
