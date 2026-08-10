//! Backend-owned text appearance state for `/out/text`.
//!
//! Text on the face is current state, not a delivery queue or a message log.
//! The backend owns one current exchange:
//!
//! - the latest settled human line;
//! - an optional rolling speech-recognition interim;
//! - the agent's current reply as it grows.
//!
//! Every subscriber receives the current state immediately and then whole-state
//! replacements. There are no message ids, client ids, cursors, acknowledgements
//! or historical catch-up. A slow or reconnecting surface may miss intermediate
//! typing states, but it converges on the same present state as every other
//! surface. The journal, not the appearance, is the conversation history.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use serde::Serialize;
use tokio::sync::watch;

/// A rolling speech-recognition partial that never settles is presentation
/// noise. Expire it in the authoritative state rather than independently in
/// every window.
const INTERIM_STALE_AFTER: Duration = Duration::from_secs(3);

/// The agent's current worded reply.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AgentText {
    pub text: String,
    /// `false` while chunks are still arriving; `true` once the current
    /// utterance boundary has closed.
    #[serde(rename = "final")]
    pub is_final: bool,
}

/// The complete text state rendered by the appearance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TextState {
    /// Latest settled human line in the current exchange.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
    /// Agent reply accumulated for the current exchange.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent: Option<AgentText>,
    /// Cumulative live STT partial, overlaid without replacing the settled
    /// exchange until it becomes final.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub interim: Option<String>,

    /// A settled human line opened the current exchange and is waiting for the
    /// first agent text turn. Internal ownership state; never sent on the wire.
    #[serde(skip)]
    awaiting_agent: bool,
    /// Whether the current reaction turn started after the latest settled
    /// human line. Internal ownership state; never sent on the wire.
    #[serde(skip)]
    agent_output_allowed: bool,
    /// Whether this reaction turn has already begun its visible reply.
    /// Internal ownership state; never sent on the wire.
    #[serde(skip)]
    agent_output_started: bool,
    /// Latest reaction turn superseded by settled human input.
    /// Internal ownership state; never sent on the wire.
    #[serde(skip)]
    blocked_through_turn: Option<u64>,
}

impl Default for TextState {
    fn default() -> Self {
        Self {
            user: None,
            agent: None,
            interim: None,
            awaiting_agent: false,
            agent_output_allowed: true,
            agent_output_started: false,
            blocked_through_turn: None,
        }
    }
}

/// Cloneable owner of the current text appearance.
#[derive(Clone)]
pub struct TextAppearance {
    tx: watch::Sender<TextState>,
    interim_generation: Arc<AtomicU64>,
}

impl TextAppearance {
    pub fn new() -> Self {
        let (tx, _) = watch::channel(TextState::default());
        Self {
            tx,
            interim_generation: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Subscribe to the present state. The receiver already contains the
    /// current snapshot; no identity or reading position is involved.
    pub fn subscribe(&self) -> watch::Receiver<TextState> {
        self.tx.subscribe()
    }

    /// Fold recognized human text into the appearance.
    ///
    /// Rolling partials are an overlay: they stop the voice and show what is
    /// currently being heard without erasing the settled exchange. A final line
    /// starts the next exchange and replaces the prior user/reply pair.
    pub fn note_user(&self, text: &str, is_final: bool, latest_started_turn: Option<u64>) {
        let text = text.trim();
        if text.is_empty() {
            return;
        }

        let generation = self.interim_generation.fetch_add(1, Ordering::Relaxed) + 1;
        if is_final {
            let text = text.to_owned();
            self.tx.send_if_modified(|state| {
                let visible_changed = state.user.as_deref() != Some(text.as_str())
                    || state.agent.is_some()
                    || state.interim.is_some()
                    || !state.awaiting_agent;
                if visible_changed {
                    state.user = Some(text);
                    state.agent = None;
                    state.interim = None;
                }
                // Even an identical settled line is a new ordering event. It
                // must supersede a reaction turn that started after the prior
                // copy, without forcing a redundant wire snapshot.
                state.awaiting_agent = true;
                state.agent_output_allowed = false;
                state.agent_output_started = false;
                if let Some(turn) = latest_started_turn {
                    state.blocked_through_turn = Some(
                        state
                            .blocked_through_turn
                            .map_or(turn, |blocked| blocked.max(turn)),
                    );
                }
                visible_changed
            });
            return;
        }

        let text = text.to_owned();
        self.tx.send_if_modified(|state| {
            if state.interim.as_deref() == Some(text.as_str()) {
                return false;
            }
            state.interim = Some(text);
            true
        });

        let appearance = self.clone();
        tokio::spawn(async move {
            tokio::time::sleep(INTERIM_STALE_AFTER).await;
            if appearance.interim_generation.load(Ordering::Relaxed) != generation {
                return;
            }
            appearance.tx.send_if_modified(|state| {
                if state.interim.is_none() {
                    return false;
                }
                state.interim = None;
                true
            });
        });
    }

    /// Record that a new reaction turn has started.
    ///
    /// This changes no visible state. It only makes output from this turn
    /// eligible. If a settled human line arrives after this boundary, it closes
    /// that eligibility and the turn's later chunks are ignored by the
    /// appearance. The reaction and journal still run to completion.
    pub fn begin_reaction_turn(&self, turn: u64) {
        self.tx.send_if_modified(|state| {
            state.agent_output_allowed = state
                .blocked_through_turn
                .map_or(true, |blocked| turn > blocked);
            state.agent_output_started = false;
            false
        });
    }

    /// Append one agent text chunk to the current reply.
    pub fn push_agent_chunk(&self, text: String) {
        if text.is_empty() {
            return;
        }
        self.tx.send_if_modified(|state| {
            if !state.agent_output_allowed {
                return false;
            }
            if !state.agent_output_started {
                if !state.awaiting_agent {
                    state.user = None;
                }
                state.agent = Some(AgentText {
                    text: String::new(),
                    is_final: false,
                });
                state.interim = None;
                state.awaiting_agent = false;
                state.agent_output_started = true;
            }
            let agent = state
                .agent
                .as_mut()
                .expect("started agent output has state");
            agent.text.push_str(&text);
            agent.is_final = false;
            true
        });
    }

    /// Mark the current agent utterance settled. More text from the same
    /// reaction turn may append later after a tool pause; that simply flips the
    /// state back to live and continues the same current reply.
    pub fn end_agent_utterance(&self) {
        self.tx.send_if_modified(|state| {
            if !state.agent_output_allowed || !state.agent_output_started {
                return false;
            }
            let Some(agent) = state.agent.as_mut() else {
                return false;
            };
            if agent.is_final {
                return false;
            }
            agent.is_final = true;
            true
        });
    }
}

impl Default for TextAppearance {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn current(rx: &watch::Receiver<TextState>) -> TextState {
        rx.borrow().clone()
    }

    #[tokio::test]
    async fn a_new_subscriber_gets_the_current_exchange_not_history() {
        let text = TextAppearance::new();
        text.note_user("first", true, None);
        text.begin_reaction_turn(0);
        text.push_agent_chunk("old answer".into());
        text.end_agent_utterance();

        text.note_user("second", true, Some(0));
        text.begin_reaction_turn(1);
        text.push_agent_chunk("current answer".into());

        let rx = text.subscribe();
        let state = current(&rx);
        assert_eq!(state.user.as_deref(), Some("second"));
        assert_eq!(
            state.agent,
            Some(AgentText {
                text: "current answer".into(),
                is_final: false,
            })
        );
        assert_eq!(state.interim, None);
    }

    #[tokio::test]
    async fn every_subscriber_observes_the_same_state() {
        let text = TextAppearance::new();
        let mut a = text.subscribe();
        let mut b = text.subscribe();

        text.note_user("hello", true, None);
        a.changed().await.unwrap();
        b.changed().await.unwrap();

        assert_eq!(current(&a), current(&b));
        assert_eq!(current(&a).user.as_deref(), Some("hello"));
    }

    #[tokio::test]
    async fn a_new_agent_turn_replaces_an_unsolicited_prior_exchange() {
        let text = TextAppearance::new();
        text.note_user("hello", true, None);
        text.begin_reaction_turn(0);
        text.push_agent_chunk("first answer".into());
        text.end_agent_utterance();

        text.begin_reaction_turn(1);
        text.push_agent_chunk("new thought".into());

        let rx = text.subscribe();
        let state = current(&rx);
        assert_eq!(state.user, None);
        assert_eq!(state.agent.unwrap().text, "new thought");
    }

    #[tokio::test]
    async fn interim_is_an_overlay_until_the_human_line_settles() {
        let text = TextAppearance::new();
        text.note_user("old question", true, None);
        text.begin_reaction_turn(0);
        text.push_agent_chunk("old answer".into());

        text.note_user("new ques", false, Some(0));
        let rx = text.subscribe();
        let state = current(&rx);
        assert_eq!(state.user.as_deref(), Some("old question"));
        assert_eq!(
            state.agent.as_ref().map(|a| a.text.as_str()),
            Some("old answer")
        );
        assert_eq!(state.interim.as_deref(), Some("new ques"));

        text.note_user("new question", true, Some(0));
        let state = current(&rx);
        assert_eq!(state.user.as_deref(), Some("new question"));
        assert_eq!(state.agent, None);
        assert_eq!(state.interim, None);
    }

    #[tokio::test]
    async fn agent_chunks_form_one_replaceable_current_reply() {
        let text = TextAppearance::new();
        text.note_user("question", true, None);
        text.begin_reaction_turn(0);
        text.push_agent_chunk("cal".into());
        text.push_agent_chunk("endar".into());
        text.end_agent_utterance();

        let rx = text.subscribe();
        assert_eq!(
            current(&rx).agent,
            Some(AgentText {
                text: "calendar".into(),
                is_final: true,
            })
        );
    }

    #[tokio::test]
    async fn a_human_line_blocks_trailing_text_from_the_prior_reaction_turn() {
        let text = TextAppearance::new();
        text.begin_reaction_turn(0);
        text.push_agent_chunk("old answer".into());

        text.note_user("new question", true, Some(0));
        text.push_agent_chunk(" stale tail".into());
        text.end_agent_utterance();

        let rx = text.subscribe();
        let state = current(&rx);
        assert_eq!(state.user.as_deref(), Some("new question"));
        assert_eq!(state.agent, None);

        text.begin_reaction_turn(1);
        text.push_agent_chunk("new answer".into());
        let state = current(&rx);
        assert_eq!(state.user.as_deref(), Some("new question"));
        assert_eq!(
            state.agent.as_ref().map(|agent| agent.text.as_str()),
            Some("new answer")
        );
    }

    #[tokio::test]
    async fn an_identical_human_line_still_blocks_the_active_reaction_turn() {
        let text = TextAppearance::new();
        text.note_user("repeat", true, None);
        text.begin_reaction_turn(0);

        text.note_user("repeat", true, Some(0));
        text.push_agent_chunk("stale answer".into());

        let rx = text.subscribe();
        let state = current(&rx);
        assert_eq!(state.user.as_deref(), Some("repeat"));
        assert_eq!(state.agent, None);
    }

    #[tokio::test]
    async fn a_delayed_old_turn_boundary_cannot_reopen_the_appearance() {
        let text = TextAppearance::new();
        text.note_user("new question", true, Some(7));

        // The binder processes the old turn boundary after the HTTP task
        // already recorded the newer human line.
        text.begin_reaction_turn(7);
        text.push_agent_chunk("stale answer".into());

        let rx = text.subscribe();
        assert_eq!(current(&rx).agent, None);

        text.begin_reaction_turn(8);
        text.push_agent_chunk("current answer".into());
        assert_eq!(
            current(&rx).agent.as_ref().map(|agent| agent.text.as_str()),
            Some("current answer")
        );
    }
}
