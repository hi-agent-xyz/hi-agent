//! Every directory this shell reads or writes, in one place, because one of
//! them has to agree with something outside this project.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

/// The engine's data directory — the whole agent, per `docs/data-dir-layout.md`.
///
/// This value must match what `directories::ProjectDirs::from("dev",
/// "human-interface", "hi-agent").data_dir()` picks on Linux, which is
/// `$XDG_DATA_HOME/hi-agent` — the qualifier and organization are Apple and
/// Windows conventions that the Linux strategy drops.
///
/// It has to be passed explicitly all the same. `default_data_dir` in
/// `src/main.rs` only reaches for the OS data directory when
/// `bundle::resources_dir()` says it is inside a macOS `.app`; everywhere else
/// it falls back to `./data`, relative to the working directory. A `.desktop`
/// launch has no meaningful working directory, so an installed engine would
/// scatter the person's memory wherever the launcher happened to start.
/// `--data-dir` is one flag and it settles that.
pub fn engine_data() -> PathBuf {
    ensure(glib::user_data_dir().join("hi-agent"))
}

/// The shell's own state, under its own name so it can never be mistaken for
/// the agent's. Split across the XDG directories rather than pooled, because
/// WebKit takes a data directory and a cache directory as two arguments and
/// putting a cache on a backed-up path is the mistake that split is for.
fn shell_dir(base: PathBuf) -> PathBuf {
    ensure(base.join("hi-agent-shell"))
}

/// `$XDG_STATE_HOME`, or the default the spec gives for it.
///
/// GLib has `g_get_user_config_dir` and friends but no binding for
/// `g_get_user_state_dir` in glib 0.22, so this is the one XDG directory the
/// shell resolves itself. The rule is the spec's, not an invention.
fn user_state_dir() -> PathBuf {
    match std::env::var_os("XDG_STATE_HOME") {
        Some(value) if !value.is_empty() => PathBuf::from(value),
        _ => glib::home_dir().join(".local/state"),
    }
}

/// Addresses and labels. Configuration: small, worth keeping, not a secret.
pub fn roster_file() -> PathBuf {
    shell_dir(glib::user_config_dir()).join("roster.json")
}

/// The shell's log. A windowed process has no console, and a person told "it
/// did not start" needs somewhere to look.
pub fn shell_log() -> PathBuf {
    shell_dir(user_state_dir()).join("shell.log")
}

/// The engine's stdout and stderr, when this shell is the one that started it.
/// An adopted engine writes wherever its unit points it.
pub fn engine_log() -> PathBuf {
    shell_dir(user_state_dir()).join("engine.log")
}

/// WebKit's persistent website data: cookies, local storage, IndexedDB.
pub fn webkit_data() -> PathBuf {
    ensure(shell_dir(glib::user_data_dir()).join("webkit"))
}

/// WebKit's cache. Separate on purpose — see [`shell_dir`].
pub fn webkit_cache() -> PathBuf {
    ensure(shell_dir(glib::user_cache_dir()).join("webkit"))
}

/// The engine binary, beside the shell.
///
/// The `.deb` puts `hi-agent` and `hi-agent-shell` in `/usr/bin`, so this
/// resolves with no configuration. `$PATH` is the fallback for a dev checkout
/// where only one of the two was built and put somewhere by hand; `None` is a
/// state the window shows rather than crashes on, and it is also the ordinary
/// state on a machine that only ever attaches to a core elsewhere.
pub fn engine_bin() -> Option<PathBuf> {
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        let beside = dir.join("hi-agent");
        if beside.is_file() {
            return Some(beside);
        }
    }
    std::env::var_os("PATH")
        .map(|paths| std::env::split_paths(&paths).collect::<Vec<_>>())
        .unwrap_or_default()
        .into_iter()
        .map(|dir| dir.join("hi-agent"))
        .find(|candidate| candidate.is_file())
}

fn ensure(path: PathBuf) -> PathBuf {
    if let Err(e) = fs::create_dir_all(&path) {
        eprintln!("hi-agent-shell: could not create {}: {e}", path.display());
    }
    path
}

/// Append a line to the shell's log, and to stderr so that `journalctl` and a
/// terminal launch both show it.
///
/// Never rotates and never fails loudly: a log that throws would take the app
/// with it for nothing. The engine writes far more than this does.
pub fn log(message: impl AsRef<str>) {
    let message = message.as_ref();
    eprintln!("hi-agent-shell: {message}");
    // `%Y-%m-%d %H:%M:%S`, and no sub-second field: `g_date_time_format` is not
    // `strftime` and does not take chrono's `%.3f`, which it rejects — silently
    // producing an error the fallback turned into a blank column. The fallback
    // says so now rather than looking like a timestamp nobody set.
    let line = format!(
        "{} {message}\n",
        glib::DateTime::now_local()
            .and_then(|now| now.format("%Y-%m-%d %H:%M:%S"))
            .unwrap_or_else(|_| "(no clock)".into())
    );
    let path = shell_log();
    let _ = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .and_then(|mut file| file.write_all(line.as_bytes()));
}

/// Open a folder in the desktop's file manager.
///
/// The engine's data directory is the whole agent, and a person who wants to
/// back it up, copy it to another machine, or read what it wrote should not
/// have to be told a path.
pub fn show_in_files(path: &Path) {
    let launcher = gtk::FileLauncher::new(Some(&gio::File::for_path(path)));
    launcher.launch(
        None::<&gtk::Window>,
        None::<&gio::Cancellable>,
        |result| {
            if let Err(e) = result {
                log(format!("could not open folder: {e}"));
            }
        },
    );
}
