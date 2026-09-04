//! Codex back-end — talks to `codex app-server` over stdio.
//!
//! One [`CodexProcess`] holds a child process and the newline-delimited JSON-RPC
//! connection to it, hosting exactly **one** [`AgentSession`] (a codex *thread*).
//! The session owns its process (drop tears it down) and one `mpsc` of streaming
//! notifications; the reaction pulls from it while the turn runs. Isolation and
//! concurrency come from running a process per session, so there is no thread-id
//! demux.
//!
//! **Why this replaced ACP.** The wire used to be `agent-client-protocol` driving
//! `node` → `claude-agent-acp` → `claude`. Three things ended it, in order of weight:
//! ACP has no system-prompt slot, so every rung's prompt was smuggled in as the first
//! *user* message underneath Claude Code's own persona; constraining the agent's own
//! tools rode a vendor `_meta` hack ACP declines to standardise; and the `node` hop
//! bought nothing. `codex app-server` takes `baseInstructions` on `thread/start` —
//! verified on the wire as the request's `instructions` field — and is a native binary.
//!
//! Two shapes differ from ACP and the code leans on both:
//!
//! - **`turn/start` returns immediately** with the turn object; the turn's *end* is the
//!   `turn/completed` notification. So a prompt's completion is read off the stream, not
//!   off the RPC response ([`SessionRun`]).
//! - **Every notification is kept verbatim** as [`SessionUpdate::Frame`]. Text and
//!   thought are projections beside it, not instead of it.

pub mod messages;
pub mod process;
pub mod tap;
pub mod thread;

pub use messages::{Folded, Message, fold};
pub use process::{CodexProcess, ProcessRegistry, SessionOpts};
pub use tap::{Dir, RawFrame, WireTap};
pub use thread::{AgentSession, PromptResult, SessionRun, SessionUpdate, StopReason, WindowFill};
