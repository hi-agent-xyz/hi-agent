//! The process. On Linux the shell owns it — `main`, the GTK main loop, and
//! everything that touches the desktop session — and the engine is a child
//! process it starts and supervises, or one already running that it adopts.
//!
//! That is the arrangement `docs/arch/topology.md` describes for an app, and
//! the one macOS is still migrating toward from the other direction. Windows
//! got there first for the same reason this does: there has never been a Linux
//! shell to migrate, so it starts in the target shape rather than arriving at
//! it.

mod core;
mod model;
mod paths;
mod ui;

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;

use model::AppModel;
use paths::log;

/// Same reverse-DNS as the macOS bundle, with the GNOME convention of a
/// CamelCase last element — it is also the `.desktop` basename and the icon
/// name.
///
/// The hyphen in `human-interface` is legal here and worth knowing why: D-Bus
/// forbids hyphens in *interface* names, not in well-known *bus* names, and
/// GApplication's own validity rule allows them. GLib escapes it to `_` when it
/// derives the object path.
const APP_ID: &str = "dev.human-interface.HiAgent";

fn main() -> glib::ExitCode {
    let app = adw::Application::builder()
        .application_id(APP_ID)
        // Uniqueness comes free with GApplication: a second launch reaches the
        // running instance over the session bus and activates it rather than
        // starting a second shell. Two shells would mean two engines on two
        // ports writing one data directory, which is "one body per person"
        // broken by accident — the same thing `AppInstance.FindOrRegisterForKey`
        // buys on Windows, without the redirect dance.
        .flags(gio::ApplicationFlags::default())
        .build();

    // Built on `startup` rather than in `main` so that the second, redundant
    // process — the one that only forwards its activation — never constructs a
    // model or touches the engine.
    let shell: Rc<RefCell<Option<Rc<ui::window::MainWindow>>>> = Rc::new(RefCell::new(None));
    let model: Rc<RefCell<Option<Rc<AppModel>>>> = Rc::new(RefCell::new(None));

    app.connect_startup({
        let shell = shell.clone();
        let model = model.clone();
        move |app| {
            log(format!("hi-agent-shell {} starting", env!("CARGO_PKG_VERSION")));
            let this = AppModel::new();
            *shell.borrow_mut() = Some(ui::window::MainWindow::new(app, this.clone()));
            *model.borrow_mut() = Some(this.clone());

            glib::spawn_future_local(async move {
                this.start().await;
            });
        }
    });

    app.connect_activate({
        let shell = shell.clone();
        // A second launch — the app grid, a `.desktop` action — means "show me
        // the agent", not "start another one".
        move |_| {
            if let Some(window) = shell.borrow().as_ref() {
                window.present();
            }
        }
    });

    // Stop an engine this shell started before the process goes.
    // `PR_SET_PDEATHSIG` is the backstop for the paths that are not orderly;
    // this is the orderly one. An adopted engine is left alone — see
    // `LocalCore::shutdown`.
    app.connect_shutdown(move |_| {
        if let Some(model) = model.borrow().as_ref() {
            model.local().shutdown();
        }
    });

    app.run()
}
