//! What the shell knows: which cores exist, which one is attached, and what to
//! show while that is being decided.
//!
//! No widget type appears here. The window reads this and renders; this never
//! reaches into the window. That separation is the same one the native surfaces
//! are supposed to have from the engine, one level down.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use crate::core::client;
use crate::core::credentials;
use crate::core::engine::LocalCore;
use crate::core::models::{CoreError, CoreSession, CoreStage, HealthState, RosterEntry};
use crate::core::roster::RosterStore;
use crate::paths::log;

const LOCAL_CORE_ID: &str = "local";

#[derive(Default)]
struct State {
    stage: Option<CoreStage>,
    detail: Option<String>,
    session: Option<CoreSession>,
    /// Bumped by every attach. A future that finds it changed after an await
    /// has been overtaken and drops what it was going to do — which is how a
    /// person can switch cores while the first one is still starting, without
    /// a lock that would have to be held across an unbounded wait.
    generation: u64,
}

pub struct AppModel {
    roster: RosterStore,
    local: Rc<LocalCore>,
    state: RefCell<State>,
    observers: RefCell<Vec<Box<dyn Fn()>>>,
}

impl AppModel {
    pub fn new() -> Rc<Self> {
        let model = Rc::new(Self {
            roster: RosterStore::default(),
            local: LocalCore::new(),
            state: RefCell::new(State {
                stage: Some(CoreStage::Connecting),
                ..State::default()
            }),
            observers: RefCell::new(Vec::new()),
        });

        let weak = Rc::downgrade(&model);
        model.local.connect_changed(move || {
            if let Some(model) = weak.upgrade() {
                model.on_local_changed();
            }
        });
        model
    }

    pub fn connect_changed(&self, observer: impl Fn() + 'static) {
        self.observers.borrow_mut().push(Box::new(observer));
    }

    pub fn stage(&self) -> CoreStage {
        self.state.borrow().stage.unwrap_or(CoreStage::Connecting)
    }

    /// A sentence for the person when [`Self::stage`] alone will not do.
    pub fn stage_detail(&self) -> Option<String> {
        self.state.borrow().detail.clone()
    }

    pub fn session(&self) -> Option<CoreSession> {
        self.state.borrow().session.clone()
    }

    pub fn entries(&self) -> Vec<RosterEntry> {
        self.roster.entries()
    }

    pub fn attached(&self) -> Option<RosterEntry> {
        self.roster.attached()
    }

    pub fn local(&self) -> &Rc<LocalCore> {
        &self.local
    }

    pub async fn start(self: &Rc<Self>) {
        self.roster.load();

        if let Some(base_url) = self.local.start().await {
            // The local entry is written every start rather than once: the port
            // can differ between runs when 12358 was taken, and a roster holding
            // yesterday's port would point the face at nothing.
            self.roster.put(RosterEntry {
                id: LOCAL_CORE_ID.into(),
                base_url,
                label: "This computer".into(),
                is_local: true,
            });
        }

        let target = self
            .roster
            .attached()
            .or_else(|| self.roster.local())
            .or_else(|| self.roster.first());
        let Some(target) = target else {
            self.set(CoreStage::Empty, self.local.failure());
            return;
        };

        self.attach(&target.id).await;
        self.clone().poll_health();
    }

    /// Make one core the attached one: get a session if it needs one, and tell
    /// the window to load it.
    pub async fn attach(self: &Rc<Self>, id: &str) {
        let Some(entry) = self.roster.find(id) else {
            self.set(CoreStage::Failed, Some("That core is no longer in the roster.".into()));
            return;
        };

        let generation = {
            let mut state = self.state.borrow_mut();
            state.generation += 1;
            state.session = None;
            state.generation
        };
        self.roster.attach(Some(id));
        self.set(CoreStage::Connecting, None);

        if entry.is_local {
            // Wait for the engine to answer before showing the face. A first run
            // provisions its whole runtime before it is useful, which is
            // minutes, so there is no timeout here — the stage says what is
            // happening and the supervisor says if it died.
            if !self.wait_for_health(&entry, generation).await {
                return;
            }
            self.install(generation, CoreSession { entry, cookie: None });
            return;
        }

        let Some(credential) = credentials::load(&entry.id).await else {
            self.set(
                CoreStage::Failed,
                Some(format!(
                    "{} has no credential on this computer. Add it again with a pairing code.",
                    entry.label
                )),
            );
            return;
        };
        if self.overtaken(generation) {
            return;
        }

        match client::exchange(&entry.base_url, &credential, &device_label()).await {
            Ok((exchange, cookie)) => {
                if let Some(rotated) = exchange.credential
                    && let Err(e) = credentials::save(&entry.id, &rotated).await
                {
                    log(format!("rotated credential not stored: {e}"));
                }
                self.install(
                    generation,
                    CoreSession {
                        entry,
                        cookie: Some(cookie),
                    },
                );
            }
            Err(e) => {
                if !self.overtaken(generation) {
                    self.set(CoreStage::Failed, Some(e.to_string()));
                }
            }
        }
    }

    /// Add a core the person typed an address and a pairing code for. The core
    /// tells a pairing code from a credential, so this presents whatever it was
    /// given and stores whatever comes back.
    pub async fn add_core(
        self: &Rc<Self>,
        address: &str,
        pairing_code: &str,
        label: &str,
    ) -> Result<(), CoreError> {
        let base_url = client::normalize_base_url(address)?;
        let (exchange, cookie) =
            client::exchange(&base_url, pairing_code.trim(), &device_label()).await?;

        // The core's surface id is the roster key, so re-adding the same core
        // updates one entry instead of growing a second.
        let entry = RosterEntry {
            id: exchange.id.clone(),
            base_url: base_url.clone(),
            label: if label.trim().is_empty() {
                glib::Uri::parse(&base_url, glib::UriFlags::NONE)
                    .ok()
                    .and_then(|uri| uri.host())
                    .map(|host| host.to_string())
                    .unwrap_or_else(|| base_url.clone())
            } else {
                label.trim().to_string()
            },
            is_local: false,
        };

        match exchange.credential {
            Some(credential) => credentials::save(&entry.id, &credential).await.map_err(|e| {
                CoreError::RequestFailed(format!(
                    "That core paired, but its credential could not be stored in the keyring: {e}"
                ))
            })?,
            // A pairing code always mints a credential; a null here means what
            // was presented already was one, for a core this machine has since
            // forgotten. Nothing to store and nothing that will work later.
            None if credentials::load(&entry.id).await.is_none() => {
                return Err(CoreError::RequestFailed(
                    "That core returned no credential. Ask it for a fresh pairing code.".into(),
                ));
            }
            None => {}
        }

        self.roster.put(entry.clone());
        self.roster.attach(Some(&entry.id));
        let generation = {
            let mut state = self.state.borrow_mut();
            state.generation += 1;
            state.generation
        };
        self.install(
            generation,
            CoreSession {
                entry,
                cookie: Some(cookie),
            },
        );
        Ok(())
    }

    /// Forget a core, and fall back to whatever is left.
    pub async fn forget(self: &Rc<Self>, id: &str) {
        let was_attached = self.roster.attached_id().as_deref() == Some(id);
        if !self.roster.remove(id) {
            return;
        }
        credentials::delete(id).await;
        if !was_attached {
            self.notify();
            return;
        }
        let next = self.roster.local().or_else(|| self.roster.first());
        match next {
            Some(entry) => self.attach(&entry.id).await,
            None => {
                self.state.borrow_mut().session = None;
                self.set(CoreStage::Empty, None);
            }
        }
    }

    /// The face met a 401. Exchange the credential again and hand the window a
    /// new session; the local core has no session to renew, so a 401 there is a
    /// real error rather than an expiry.
    pub async fn renew_session(self: &Rc<Self>) {
        match self.roster.attached() {
            Some(entry) if !entry.is_local => self.attach(&entry.id).await,
            _ => self.set(
                CoreStage::Failed,
                Some("The agent refused a request from its own face.".into()),
            ),
        }
    }

    /// The window says the face painted.
    pub fn report_ready(&self) {
        self.set(CoreStage::Ready, None);
    }

    /// The window says the load failed.
    pub fn report_failure(&self, message: impl Into<String>) {
        self.set(CoreStage::Failed, Some(message.into()));
    }

    fn install(&self, generation: u64, session: CoreSession) {
        if self.overtaken(generation) {
            return;
        }
        self.state.borrow_mut().session = Some(session);
        self.set(CoreStage::Connecting, None);
    }

    fn overtaken(&self, generation: u64) -> bool {
        self.state.borrow().generation != generation
    }

    /// Returns false when this attach was overtaken while waiting.
    async fn wait_for_health(&self, entry: &RosterEntry, generation: u64) -> bool {
        loop {
            if self.overtaken(generation) {
                return false;
            }
            if client::health(&entry.base_url).await == HealthState::Here {
                return !self.overtaken(generation);
            }
            match self.local.failure() {
                Some(failure) => self.set(CoreStage::Failed, Some(failure)),
                None => self.set(CoreStage::Connecting, Some("Starting the agent…".into())),
            }
            glib::timeout_future(Duration::from_secs(1)).await;
        }
    }

    /// Poll the attached core. Not a heartbeat for the core's benefit — it is
    /// how the window knows to stop showing a face that is no longer answering.
    fn poll_health(self: Rc<Self>) {
        glib::spawn_future_local(async move {
            loop {
                glib::timeout_future(Duration::from_secs(10)).await;
                let Some(entry) = self.roster.attached() else {
                    continue;
                };
                match client::health(&entry.base_url).await {
                    HealthState::Here => {
                        if self.stage() == CoreStage::Waiting {
                            self.set(CoreStage::Connecting, None);
                        }
                    }
                    _ if matches!(self.stage(), CoreStage::Ready | CoreStage::Connecting) => {
                        self.set(
                            CoreStage::Waiting,
                            Some(if entry.is_local {
                                "The agent is not answering.".into()
                            } else {
                                format!("{} is not answering.", entry.label)
                            }),
                        );
                    }
                    _ => {}
                }
            }
        });
    }

    fn on_local_changed(&self) {
        if let Some(failure) = self.local.failure()
            && self.roster.attached().is_some_and(|entry| entry.is_local)
        {
            self.set(CoreStage::Failed, Some(failure));
        }
    }

    fn set(&self, stage: CoreStage, detail: Option<String>) {
        {
            let mut state = self.state.borrow_mut();
            state.stage = Some(stage);
            state.detail = detail;
        }
        self.notify();
    }

    fn notify(&self) {
        for observer in self.observers.borrow().iter() {
            observer();
        }
    }
}

/// What the core calls this device in its list of authorized surfaces. The host
/// name, because that is what a person recognises when they come to revoke one.
fn device_label() -> String {
    glib::host_name().to_string()
}
