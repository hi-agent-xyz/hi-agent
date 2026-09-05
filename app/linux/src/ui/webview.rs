//! The core's face, in WebKitGTK.
//!
//! Everything unusual in here is one of two things: a WebKit default that is
//! wrong for a full-window app face, or something the other clients get from
//! their platform and this one has to build. The Swift, Kotlin and C# files of
//! the same name are the reference — where this deviates, it says so.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk::gdk;
use gtk::prelude::*;
use webkit::prelude::*;

use crate::core::models::{CoreSession, CoreStage};
use crate::model::AppModel;
use crate::paths::{log, webkit_cache, webkit_data};

/// The face's own `fetch` can meet a 401 long after the page loaded — a session
/// that expired while the machine was asleep — and that is invisible to every
/// navigation event. This is the `WKUserScript` /
/// `AddScriptToExecuteOnDocumentCreated` equivalent.
///
/// Unlike WebView2's, WebKit's user scripts take an allow-list of URL patterns,
/// so the injection is origin-scoped by the engine rather than by a check the
/// script does on itself. The Rust side checks the origin again on arrival
/// anyway; what the script can do is bounded regardless, being one fixed
/// message asking the shell to re-exchange a credential the page never sees.
const SESSION_OBSERVER: &str = r#"
(() => {
  if (window.__hiAgentSessionObserverInstalled) return;
  window.__hiAgentSessionObserverInstalled = true;
  const origin = window.location.origin;
  const originalFetch = window.fetch.bind(window);
  window.fetch = async (...args) => {
    const response = await originalFetch(...args);
    if (response.status === 401 && window.location.origin === origin) {
      window.webkit.messageHandlers.hiAgent.postMessage('unauthorized');
    }
    return response;
  };
})();
"#;

pub struct CoreWebView {
    view: webkit::WebView,
    model: Rc<AppModel>,
    installed: RefCell<Option<CoreSession>>,
    renewal_requested: Cell<bool>,
}

impl CoreWebView {
    pub fn new(model: Rc<AppModel>) -> Rc<Self> {
        // Persistent, so the face's own local storage survives a restart the
        // way it does in a browser. The session cookie is not what persists —
        // it is deleted and reinstalled on every attach.
        let network = webkit::NetworkSession::new(
            webkit_data().to_str(),
            webkit_cache().to_str(),
        );

        let content = webkit::UserContentManager::new();
        content.register_script_message_handler("hiAgent", None);

        let view = webkit::WebView::builder()
            .network_session(&network)
            .user_content_manager(&content)
            .vexpand(true)
            .hexpand(true)
            .build();

        // Spelled out because `WidgetExt::settings` (the GtkSettings for the
        // display) and `WebViewExt::settings` (the WebKitSettings for this
        // view) are both in scope and both apply. The latter is nullable in C,
        // so it arrives as an `Option`; the fallback attaches what it creates
        // rather than defaulting to a detached object, which would leave every
        // line below silently applying to nothing.
        let settings = WebViewExt::settings(&view).unwrap_or_else(|| {
            let settings = webkit::Settings::new();
            view.set_settings(&settings);
            settings
        });
        // The Linux spelling of the media-gesture trap that cost both phones a
        // microphone and cost Windows a browser argument. WebKit gates
        // `AudioContext` — the graph the mic runs through, and the graph the
        // agent's voice comes out of — behind a user gesture, and the face
        // builds that context on load where there is no gesture. Without this
        // the camera works, the mic is silently dead, and the agent never
        // speaks. See `CoreWebView.swift`
        // (`mediaTypesRequiringUserActionForPlayback`), `CoreWebView.kt`
        // (`mediaPlaybackRequiresUserGesture`) and `CoreWebView.cs`
        // (`--autoplay-policy=no-user-gesture-required`).
        settings.set_media_playback_requires_user_gesture(false);
        settings.set_enable_developer_extras(cfg!(debug_assertions));

        let face = Rc::new(Self {
            view,
            model,
            installed: RefCell::new(None),
            renewal_requested: Cell::new(false),
        });
        face.wire(&content);
        face
    }

    pub fn widget(&self) -> &webkit::WebView {
        &self.view
    }

    /// Let the window's colour show until the face paints, so opening in dark
    /// appearance does not flash a white page.
    pub fn set_background(&self, colour: &gdk::RGBA) {
        self.view.set_background_color(colour);
    }

    fn wire(self: &Rc<Self>, content: &webkit::UserContentManager) {
        // Only one message is ever sent, so the payload is not read: arrival on
        // this handler *is* the message. What is checked is where it came from.
        let this = Rc::downgrade(self);
        content.connect_script_message_received(Some("hiAgent"), move |_, _| {
            if let Some(this) = this.upgrade()
                && this.is_trusted(this.view.uri().as_deref())
            {
                this.request_renewal();
            }
        });

        let this = Rc::downgrade(self);
        self.view.connect_permission_request(move |_, request| {
            let Some(this) = this.upgrade() else {
                return false;
            };
            this.on_permission_request(request)
        });

        let this = Rc::downgrade(self);
        self.view
            .connect_decide_policy(move |_, decision, kind| {
                let Some(this) = this.upgrade() else {
                    return false;
                };
                this.on_decide_policy(decision, kind)
            });

        let this = Rc::downgrade(self);
        self.view.connect_load_changed(move |view, event| {
            if event != webkit::LoadEvent::Finished {
                return;
            }
            let Some(this) = this.upgrade() else {
                return;
            };
            this.on_load_finished(view);
        });

        let this = Rc::downgrade(self);
        self.view
            .connect_load_failed(move |_, _, _, error| {
                if let Some(this) = this.upgrade() {
                    this.model.report_failure(describe(error));
                }
                // Handled: the stage shows the sentence, so WebKit must not
                // also paint its own error page inside the app's chrome.
                true
            });

        // A certificate WebKit will not accept never reaches `load-failed`, so
        // refusing it is a separate signal. Returning true means "handled, do
        // not continue" — the shell does not offer to proceed anyway, because
        // the thing on the other end is supposed to be the person's own agent.
        let this = Rc::downgrade(self);
        self.view
            .connect_load_failed_with_tls_errors(move |_, _, _, _| {
                if let Some(this) = this.upgrade() {
                    this.model
                        .report_failure("The core's secure connection could not be verified.");
                }
                true
            });

        let this = Rc::downgrade(self);
        self.view.connect_web_process_terminated(move |_, reason| {
            log(format!("web process terminated: {reason:?}"));
            if let Some(this) = this.upgrade() {
                this.model
                    .report_failure("The face stopped responding. Try again.");
            }
        });
    }

    /// Called on every render. Loads the model's session if it is not the one
    /// already loaded, and does nothing at all otherwise — a reload on each
    /// state change would restart the conversation's stream for no reason.
    pub fn sync(self: &Rc<Self>) {
        let Some(session) = self.model.session() else {
            return;
        };
        if self
            .installed
            .borrow()
            .as_ref()
            .is_some_and(|current| current.same_as(&session))
        {
            return;
        }

        *self.installed.borrow_mut() = Some(session.clone());
        self.renewal_requested.set(false);
        self.install_script(&session);

        let this = self.clone();
        glib::spawn_future_local(async move {
            let manager = this
                .view
                .network_session()
                .and_then(|network| network.cookie_manager());
            if let Some(manager) = manager {
                // Empty the jar before filling it, so it never holds two cores'
                // sessions at once. Relayed cores share one origin —
                // `hi-agent.xyz/ana` and `hi-agent.xyz/bob` are the same site to
                // a cookie store, and `Path=` decides only what is *sent* where,
                // not what is readable. Required by the App section of
                // `docs/arch/topology.md`, which is what lets the session live
                // in the page at all.
                //
                // One at a time, because WebKitGTK 6.0 has no bulk delete —
                // `webkit_cookie_manager_delete_all_cookies` was dropped with
                // the 2.x API and `WebKitWebsiteDataManager` has no `clear` in
                // this binding. Windows and the phones each get this in a line.
                match manager.all_cookies_future().await {
                    Ok(existing) => {
                        for cookie in &existing {
                            if let Err(e) = manager.delete_cookie_future(cookie).await {
                                log(format!("cookie not deleted: {e}"));
                            }
                        }
                    }
                    Err(e) => log(format!("cookie jar not read: {e}")),
                }
                if let Some(cookie) = &session.cookie
                    && let Err(e) = manager.add_cookie_future(cookie).await
                {
                    log(format!("session cookie not installed: {e}"));
                    this.model
                        .report_failure("The session could not be handed to the face.");
                    return;
                }
            }
            this.view.load_uri(&session.entry.base_url);
        });
    }

    /// Re-scope the session observer to the core about to be loaded. WebKit
    /// holds user scripts on the content manager rather than on a navigation,
    /// so switching cores has to remove the previous core's allow-list.
    fn install_script(&self, session: &CoreSession) {
        let Some(content) = self.view.user_content_manager() else {
            return;
        };
        content.remove_all_scripts();
        let pattern = format!("{}*", session.entry.base_url.trim_end_matches('/'));
        content.add_script(&webkit::UserScript::new(
            SESSION_OBSERVER,
            webkit::UserContentInjectedFrames::TopFrame,
            webkit::UserScriptInjectionTime::Start,
            &[pattern.as_str()],
            &[],
        ));
    }

    /// Exact scheme, host and port — the same rule as the iOS `isTrusted`.
    fn is_trusted(&self, uri: Option<&str>) -> bool {
        let (Some(uri), Some(session)) = (uri, self.installed.borrow().clone()) else {
            return false;
        };
        let (Ok(target), Ok(expected)) = (
            glib::Uri::parse(uri, glib::UriFlags::NONE),
            glib::Uri::parse(&session.entry.base_url, glib::UriFlags::NONE),
        ) else {
            return false;
        };
        target.scheme().eq_ignore_ascii_case(&expected.scheme())
            && target
                .host()
                .zip(expected.host())
                .is_some_and(|(a, b)| a.eq_ignore_ascii_case(&b))
            && target.port() == expected.port()
    }

    /// Camera and microphone, granted only to the attached core's exact origin.
    ///
    /// An unhandled `WebKitUserMediaPermissionRequest` is *denied*, so handling
    /// this signal is not a refinement — it is the entire mic and camera
    /// implementation. There is no TCC and no per-app permission grant to check
    /// first, so origin is the whole question at this rung.
    fn on_permission_request(&self, request: &webkit::PermissionRequest) -> bool {
        // Capture and paste, and nothing else. Geolocation, notifications,
        // pointer lock and the rest fall through to the deny below rather than
        // being enumerated — a permission this shell has not thought about is
        // one the face does not get.
        let wanted = request.is::<webkit::UserMediaPermissionRequest>()
            || request.is::<webkit::ClipboardPermissionRequest>();
        if wanted && self.is_trusted(self.view.uri().as_deref()) {
            request.allow();
        } else {
            request.deny();
        }
        true
    }

    /// A link out of the core's own origin leaves for the browser.
    ///
    /// The face is the whole window with no address bar, so an off-origin page
    /// would render inside the app's chrome wearing its identity. The session
    /// cookie is host-scoped and does not travel, so this is about what the
    /// person is being shown rather than about what leaks.
    fn on_decide_policy(
        &self,
        decision: &webkit::PolicyDecision,
        kind: webkit::PolicyDecisionType,
    ) -> bool {
        let uri = match kind {
            webkit::PolicyDecisionType::NavigationAction
            | webkit::PolicyDecisionType::NewWindowAction => decision
                .downcast_ref::<webkit::NavigationPolicyDecision>()
                .and_then(|d| d.navigation_action())
                .and_then(|action| action.request())
                .and_then(|request| request.uri()),
            // A response decision is about what to do with a body already
            // being fetched from an origin the navigation check let through.
            _ => return false,
        };

        if self.is_trusted(uri.as_deref()) && kind == webkit::PolicyDecisionType::NavigationAction {
            return false;
        }
        decision.ignore();
        if let Some(uri) = uri {
            open_externally(&uri);
        }
        true
    }

    /// A main-frame status is readable, as on Windows and unlike Android: the
    /// main resource's response carries the status code, so a 401 is met by
    /// re-exchanging the credential rather than by rendering an "unauthorized"
    /// body.
    fn on_load_finished(self: &Rc<Self>, view: &webkit::WebView) {
        let status = view
            .main_resource()
            .and_then(|resource| resource.response())
            .map_or(0, |response| response.status_code());

        match status {
            // No response on the main resource at all — nothing was fetched
            // over HTTP, so there is no status to judge and the page is
            // whatever it is.
            0 => self.model.report_ready(),
            401 => self.request_renewal(),
            200..=399 => {
                self.renewal_requested.set(false);
                self.model.report_ready();
            }
            other => self
                .model
                .report_failure(format!("The core answered with HTTP {other}.")),
        }
    }

    fn request_renewal(self: &Rc<Self>) {
        if self.renewal_requested.replace(true) {
            return;
        }
        let model = self.model.clone();
        glib::spawn_future_local(async move {
            model.renew_session().await;
        });
    }
}

/// Whether the face is what the window should be showing at all.
pub fn shows_face(stage: CoreStage) -> bool {
    stage == CoreStage::Ready
}

fn open_externally(uri: &str) {
    let launcher = gtk::UriLauncher::new(uri);
    launcher.launch(
        None::<&gtk::Window>,
        None::<&gio::Cancellable>,
        |result| {
            if let Err(e) = result {
                log(format!("could not open link: {e}"));
            }
        },
    );
}

/// A sentence for the person, from whichever error domain WebKit surfaced.
///
/// A failed certificate is not among them: WebKit routes that to
/// `load-failed-with-tls-errors` rather than to `load-failed`, which is why the
/// shell connects both.
fn describe(error: &glib::Error) -> String {
    if let Some(resolver) = error.kind::<gio::ResolverError>() {
        return match resolver {
            gio::ResolverError::NotFound => "The core address could not be found.".into(),
            _ => "The core address could not be looked up.".into(),
        };
    }
    if let Some(network) = error.kind::<webkit::NetworkError>() {
        return match network {
            webkit::NetworkError::Cancelled => "The connection to the core was lost.".into(),
            webkit::NetworkError::Transport => "Nothing answered at the core's address.".into(),
            webkit::NetworkError::UnknownProtocol => {
                "That core address is not one this app can open.".into()
            }
            _ => "The core could not be reached.".into(),
        };
    }
    "The core could not be reached.".into()
}
