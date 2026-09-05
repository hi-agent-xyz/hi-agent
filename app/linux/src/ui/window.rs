//! The face's frame, and the app's only window. It shows the core's page or a
//! sentence about why it cannot yet — never both.

use std::rc::Rc;

use adw::prelude::*;
use gtk::gdk;
use gtk::glib::clone;

use crate::core::models::CoreStage;
use crate::model::AppModel;
use crate::paths::{engine_data, shell_log, show_in_files};

use super::pair;
use super::webview::{CoreWebView, shows_face};

/// `--bg-1`, the token the face paints across the strip directly below the
/// header bar (`src/appearance/web/src/ui/global.css`).
///
/// A default-coloured header draws a seam across the top of the window in
/// exactly the place a person reads as the app's edge. macOS solves this in
/// `apply_face_theme` and Windows in `ApplyTitleBarTheme`; this is the third
/// copy, and all three have to be changed together when the token moves.
const BG_1_LIGHT: &str = "#ffffff";
const BG_1_DARK: &str = "#2b2720";

pub struct MainWindow {
    window: adw::ApplicationWindow,
    model: Rc<AppModel>,
    face: Rc<CoreWebView>,
    stack: gtk::Stack,
    status: adw::StatusPage,
    spinner: adw::Spinner,
    retry: gtk::Button,
    add_core: gtk::Button,
    cores: gio::Menu,
}

impl MainWindow {
    pub fn new(app: &adw::Application, model: Rc<AppModel>) -> Rc<Self> {
        let window = adw::ApplicationWindow::builder()
            .application(app)
            .title("Hi Agent")
            .default_width(1100)
            .default_height(760)
            .build();

        let face = CoreWebView::new(model.clone());

        let spinner = adw::Spinner::builder()
            .width_request(32)
            .height_request(32)
            .build();
        let retry = gtk::Button::with_label("Try again");
        let add_core = gtk::Button::with_label("Add a core…");
        add_core.add_css_class("suggested-action");

        let buttons = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(8)
            .halign(gtk::Align::Center)
            .build();
        buttons.append(&retry);
        buttons.append(&add_core);

        let stage_content = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(16)
            .halign(gtk::Align::Center)
            .build();
        stage_content.append(&spinner);
        stage_content.append(&buttons);

        // What is shown instead of the face, and only ever instead of it:
        // starting up, waiting, or a sentence about what went wrong. Not an
        // overlay on a live face — a half-loaded agent behind a spinner reads
        // as a hung one.
        let status = adw::StatusPage::builder()
            .title("Starting the agent…")
            .child(&stage_content)
            .build();

        let stack = gtk::Stack::builder()
            .transition_type(gtk::StackTransitionType::Crossfade)
            .build();
        stack.add_named(&status, Some("stage"));
        stack.add_named(face.widget(), Some("face"));

        // Stock GNOME has no tray, so this menu is the app's whole presence
        // besides the window itself. The macOS twin is the menu-bar item in
        // `macos_tray.rs` and the Windows one is `TrayIcon.cs`; the list is
        // deliberately the same short one.
        let cores = gio::Menu::new();
        let menu = gio::Menu::new();
        menu.append_section(None, &cores);
        let places = gio::Menu::new();
        places.append(Some("Add a core…"), Some("win.add-core"));
        menu.append_section(None, &places);
        let files = gio::Menu::new();
        files.append(Some("Open the agent's folder"), Some("win.open-data"));
        files.append(Some("Open the app's logs"), Some("win.open-logs"));
        menu.append_section(None, &files);
        let quit = gio::Menu::new();
        quit.append(Some("Quit Hi Agent"), Some("win.quit"));
        menu.append_section(None, &quit);

        let header = adw::HeaderBar::new();
        header.pack_end(
            &gtk::MenuButton::builder()
                .icon_name("open-menu-symbolic")
                .tooltip_text("Main menu")
                .menu_model(&menu)
                .build(),
        );

        let layout = adw::ToolbarView::builder().content(&stack).build();
        layout.add_top_bar(&header);
        window.set_content(Some(&layout));

        let this = Rc::new(Self {
            window,
            model,
            face,
            stack,
            status,
            spinner,
            retry,
            add_core,
            cores,
        });
        this.wire();
        this.apply_theme();
        this.render();
        this
    }

    pub fn present(&self) {
        self.window.present();
    }

    fn wire(self: &Rc<Self>) {
        let actions = gio::SimpleActionGroup::new();

        let retry = gio::SimpleAction::new("retry", None);
        retry.connect_activate(clone!(
            #[weak(rename_to = this)]
            self,
            move |_, _| this.retry()
        ));
        actions.add_action(&retry);

        let add = gio::SimpleAction::new("add-core", None);
        add.connect_activate(clone!(
            #[weak(rename_to = this)]
            self,
            move |_, _| pair::present(&this.window, this.model.clone())
        ));
        actions.add_action(&add);

        // One action with a string target rather than one action per core: the
        // roster changes while the window is open.
        let attach = gio::SimpleAction::new("attach", Some(glib::VariantTy::STRING));
        attach.connect_activate(clone!(
            #[weak(rename_to = this)]
            self,
            move |_, target| {
                let Some(id) = target.and_then(|t| t.str()).map(str::to_string) else {
                    return;
                };
                let model = this.model.clone();
                glib::spawn_future_local(async move { model.attach(&id).await });
            }
        ));
        actions.add_action(&attach);

        // Destructive and not undoable — the credential goes with the entry,
        // and getting the core back means a fresh pairing code from it.
        let forget = gio::SimpleAction::new("forget", Some(glib::VariantTy::STRING));
        forget.connect_activate(clone!(
            #[weak(rename_to = this)]
            self,
            move |_, target| {
                let Some(id) = target.and_then(|t| t.str()).map(str::to_string) else {
                    return;
                };
                let Some(entry) = this.model.entries().into_iter().find(|e| e.id == id) else {
                    return;
                };
                let confirm = adw::AlertDialog::new(
                    Some(&format!("Forget {}?", entry.label)),
                    Some(
                        "This computer will drop its credential for that core. \
                         Adding it again needs a new pairing code from it.",
                    ),
                );
                confirm.add_response("cancel", "Cancel");
                confirm.add_response("forget", "Forget");
                confirm.set_response_appearance("forget", adw::ResponseAppearance::Destructive);
                confirm.set_default_response(Some("cancel"));
                confirm.set_close_response("cancel");
                confirm.connect_response(
                    None,
                    clone!(
                        #[weak(rename_to = this)]
                        this,
                        move |_, response| {
                            if response != "forget" {
                                return;
                            }
                            let (model, id) = (this.model.clone(), id.clone());
                            glib::spawn_future_local(async move { model.forget(&id).await });
                        }
                    ),
                );
                confirm.present(Some(&this.window));
            }
        ));
        actions.add_action(&forget);

        let open_data = gio::SimpleAction::new("open-data", None);
        open_data.connect_activate(|_, _| show_in_files(&engine_data()));
        actions.add_action(&open_data);

        let open_logs = gio::SimpleAction::new("open-logs", None);
        open_logs.connect_activate(|_, _| {
            if let Some(dir) = shell_log().parent() {
                show_in_files(dir);
            }
        });
        actions.add_action(&open_logs);

        // Closing the window quits the shell, and that is the honest shape on
        // this platform rather than an omission. macOS keeps the agent alive
        // with a menu-bar item and stock GNOME has no tray to retreat into, so
        // a held process with no window would be invisible and unquittable.
        // Liveness with no window open is the `systemd --user` unit's job —
        // and because the shell adopts an engine it finds rather than starting
        // a second, quitting here leaves a unit-managed engine running.
        let quit = gio::SimpleAction::new("quit", None);
        quit.connect_activate(clone!(
            #[weak(rename_to = this)]
            self,
            move |_, _| this.window.close()
        ));
        actions.add_action(&quit);

        self.window.insert_action_group("win", Some(&actions));

        self.retry.connect_clicked(clone!(
            #[weak(rename_to = this)]
            self,
            move |_| this.retry()
        ));
        self.add_core.connect_clicked(clone!(
            #[weak(rename_to = this)]
            self,
            move |_| pair::present(&this.window, this.model.clone())
        ));

        self.model.connect_changed(clone!(
            #[weak(rename_to = this)]
            self,
            move || this.render()
        ));

        adw::StyleManager::default().connect_dark_notify(clone!(
            #[weak(rename_to = this)]
            self,
            move |_| this.apply_theme()
        ));
    }

    fn retry(self: &Rc<Self>) {
        let Some(entry) = self.model.attached() else {
            return;
        };
        let model = self.model.clone();
        glib::spawn_future_local(async move { model.attach(&entry.id).await });
    }

    fn render(self: &Rc<Self>) {
        let stage = self.model.stage();
        let detail = self.model.stage_detail();

        self.stack
            .set_visible_child_name(if shows_face(stage) { "face" } else { "stage" });
        self.spinner
            .set_visible(matches!(stage, CoreStage::Connecting | CoreStage::Waiting));
        self.retry
            .set_visible(matches!(stage, CoreStage::Failed | CoreStage::Waiting));
        self.add_core
            .set_visible(matches!(stage, CoreStage::Empty | CoreStage::Failed));

        self.status.set_title(match stage {
            CoreStage::Empty => "No agent yet",
            CoreStage::Connecting => "Starting the agent…",
            CoreStage::Waiting => "Waiting for the agent",
            CoreStage::Failed => "The agent could not be reached",
            CoreStage::Ready => "",
        });
        self.status.set_description(detail.as_deref());

        // Only worth listing when there is a choice to make. One core is not a
        // list, it is the agent.
        self.cores.remove_all();
        let entries = self.model.entries();
        let attached = self.model.attached();
        if entries.len() > 1 {
            let attached_id = attached.as_ref().map(|entry| &entry.id);
            for entry in &entries {
                let mark = if Some(&entry.id) == attached_id {
                    "● "
                } else {
                    "   "
                };
                self.cores.append(
                    Some(&format!("{mark}{}", entry.label)),
                    Some(&format!("win.attach::{}", entry.id)),
                );
            }
        }
        // The local engine cannot be forgotten — it is this machine, and
        // removing it would leave the shell supervising a process it has no
        // entry for.
        if let Some(entry) = attached.filter(|entry| !entry.is_local) {
            self.cores.append(
                Some(&format!("Forget {}…", entry.label)),
                Some(&format!("win.forget::{}", entry.id)),
            );
        }

        self.face.sync();
    }

    fn apply_theme(&self) {
        let dark = adw::StyleManager::default().is_dark();
        let colour = if dark { BG_1_DARK } else { BG_1_LIGHT };

        let css = gtk::CssProvider::new();
        css.load_from_string(&format!(
            "headerbar, .background {{ background-color: {colour}; }}"
        ));
        if let Some(display) = gdk::Display::default() {
            gtk::style_context_add_provider_for_display(
                &display,
                &css,
                gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
            );
        }

        if let Ok(rgba) = colour.parse::<gdk::RGBA>() {
            self.face.set_background(&rgba);
        }
    }
}
