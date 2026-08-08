//! Minimal MCP server — the tool carrier between the mind and the reaction module.
//!
//! The reaction session (and its workers) reach this over the `mcp_servers` block in their thread config
//! attachment as an HTTP MCP endpoint (`/mcp`). It speaks just enough of the MCP
//! "Streamable HTTP" transport to serve tools: a JSON-RPC *request* gets a single
//! `application/json` response, a *notification* gets `202 Accepted`, and the GET
//! SSE stream is declined (`405`) since we never push server-initiated messages.
//! No MCP transport session ids — each agent session opens its own connection and
//! identifies its role and agent-session id on every call via headers, so the
//! transport stays stateless here.
//!
//! This module is transport-free: it turns a parsed JSON-RPC message plus the
//! routing identity (role/session id from headers) into an [`McpReply`]. The
//! HTTP glue lives in `crate::foundation::server::mcp`. Tool calls are forwarded to the right
//! reaction loop through the [`ToolRegistry`]; see [`crate::body::reaction::tools`].

use serde_json::{Value, json};

use base64::Engine as _;

use std::path::Path;
use std::sync::Mutex;

use bytes::Bytes;
use chrono::{DateTime, Utc};

use crate::body::capabilities::{image_gen, video_gen, view_render};
use crate::body::reaction::{LoopControl, ToolOwner, ToolRegistry};
use crate::foundation::observatory::{EventKind, Observatory};
use crate::foundation::registry;
use crate::identity::WorkerType;
use crate::mind::memory::people_vectors;
use crate::foundation::server::PartialMinute;
use crate::types::ViewTraits;

/// MCP protocol version we advertise when the client doesn't pin one. We echo the
/// client's requested version when present, so this is only the fallback.
const PROTOCOL_VERSION: &str = "2025-06-18";

/// What the HTTP layer should send back. `Json` is a JSON-RPC response body;
/// `Accepted` is the empty 202 for notifications/responses.
pub enum McpReply {
    Json(Value),
    Accepted,
}

/// The tool surfaces, selected by the `X-HI-Role` header. The reaction gets only
/// `show` (it speaks via plain message text, not a tool); a worker gets the
/// work tools but no voice; reflection reads/writes derived memory. The `_` fallback
/// is the legacy agentic reaction's full toolset, kept for untagged sessions.
/// The `say` tool — Reaction's voice.
///
/// Speech is a **call, not message text**, and that is the whole point: a call returns.
/// The host can hold an utterance until the room is right, queue it behind another, or
/// refuse it — and Reaction finds out which. Text streamed into the transcript is
/// fire-and-forget and leaves nowhere for that decision to live.
fn say_tool() -> Value {
    tool(
        "say",
        "Speak to the person. Everything you want said aloud goes through this tool — \
         plain text you write is NOT spoken. Call it with one natural chunk at a time, \
         keeping each call under about 240 characters; an overlong call returns too_long \
         and is not sent. Several accepted calls in a turn are spoken in order. To stay \
         silent, don't call it at all. It tells you where the words actually landed — \
         aloud, on screen only, or waiting for them to come back — so you can judge \
         whether a spoken line was worth spending.",
        json!({
            "type": "object",
            "properties": { "text": { "type": "string", "description": "What to say, as natural spoken language (no markdown)." } },
            "required": ["text"],
        }),
    )
}

/// `SendMessage` — the one verb between agents.
///
/// One direction, no reply. A reply is this same call going the other way, which is why
/// the sender is stamped host-side from the calling session rather than passed in.
fn send_message_tool() -> Value {
    tool(
        "send_message",
        "Send a message to another agent session. One direction — it does not wait for a \
         reply, and the return value only tells you whether it was delivered. If you want an \
         answer, the other side sends you one the same way; your identity travels with the \
         message so it knows where to reach you. `to` is always a **session id** — a number. \
         A worker's comes back from `create_worker`; a message you received carries its \
         sender's; and everyone else you may reach is listed in your window under \"Who you \
         can reach right now\", each with its id. Nobody is reachable by name.",
        json!({
            "type": "object",
            "properties": {
                "to": { "type": "string", "description": "A session id (a number)." },
                "message": { "type": "string", "description": "What you want them to know, in plain words." },
            },
            "required": ["to", "message"],
        }),
    )
}

fn create_worker_tool() -> Value {
    tool(
        "create_worker",
        "Start a working session to carry out a job, and get back its session id. It runs \
         with the full toolset and no voice of its own; it reports to you and to nobody else. \
         Send it the brief with `send_message`, ask how it is doing with `session_status`, and \
         read what it has produced with `session_messages`.",
        json!({
            "type": "object",
            "properties": {
                "task": { "type": "string", "description": "A self-contained description of the work." },
                "type": {
                    "type": "string",
                    "enum": WorkerType::ALL.iter().map(|t| t.as_str()).collect::<Vec<_>>(),
                    "default": WorkerType::default().as_str(),
                    "description": "What kind of session to start. `general` for almost everything \
                                    — reach for a specialist only when the job plainly is one: \
                                    `view-builder` to make something to put on screen, \
                                    `view-reviewer` to render one and judge it before it ships, \
                                    `decision-maker` to get a call made so work can continue \
                                    without the person, `file-filer` to put a handed-over file \
                                    into the drive.",
                },
            },
            "required": ["task"],
        }),
    )
}

fn session_status_tool() -> Value {
    tool(
        "session_status",
        "How a session you created is doing — whether it is working right now, what it is \
         on, how many turns it has taken. Costs you nothing but a line, so check it freely; \
         it deliberately carries none of the session's actual output.",
        json!({
            "type": "object",
            "properties": { "id": { "type": "string", "description": "The session id." } },
            "required": ["id"],
        }),
    )
}

fn session_messages_tool() -> Value {
    tool(
        "session_messages",
        "What a session you created has actually said, most recent last. This is real \
         reading and it costs context, so reach for it when you want the substance — when it \
         has finished, or when someone is asking after progress — rather than as a routine \
         check.",
        json!({
            "type": "object",
            "properties": { "id": { "type": "string", "description": "The session id." } },
            "required": ["id"],
        }),
    )
}

/// `review_view` — render a saved view in a real browser and hand back both the
/// picture and the page's own account of what went wrong.
///
/// `docs/arch/foundation.md` is blunt about the rule this exists for: *"the command
/// exited zero" is not "the thing worked"; an artifact is not shipped until it has been
/// looked at.* The capability behind this has been built and browser-proven since
/// `47b7f90` and had **zero callers** — a reviewer with no way to render, because
/// rendering is not something a session can improvise: a compiled view keeps its bare
/// imports unresolved on purpose, so it only runs inside the host page that carries our
/// import map.
///
/// Deliberately returns the problems *and* the pixels. The commonest real defect is a
/// view that "renders" as a blank white page because an import failed to resolve, and
/// pixels alone report that as success.
///
/// Renders at the frame the desktop window is currently showing
/// ([`view_render::stage_frame`]). `width`/`height` override it, for the one honest
/// reason a builder has to ask for another size: the person can resize the window, so a
/// composition that only holds at one frame is worth catching before it ships.
fn review_view_tool() -> Value {
    tool(
        "review_view",
        "Render a saved view in a real browser and look at it. Returns a verdict, any \
         errors the page reported, and a screenshot of each theme — so you can see what \
         you actually made rather than trusting that it compiled. It renders at the size \
         the person's window is showing right now, so the screenshot is the frame they \
         have. Use it on anything you are about to hand over as a view, and again after a \
         fix. Compare the light and dark frames: anything that vanishes or turns \
         unreadable in one of them is a colour that only works in the other.",
        json!({
            "type": "object",
            "properties": {
                "ref": { "type": "string", "description": "The view's ref, e.g. `project/name`." },
                "theme": { "type": "string", "enum": ["light", "dark"], "description": "Optional: render only this theme. Omit to get both, which is what you want unless you are re-checking one." },
                "lang": { "type": "string", "description": "Optional: render as if the person's language were this (e.g. `en`, `zh-Hans`). Only matters for a view that ships copy in more than one language." },
                "width": { "type": "integer", "description": "Optional: render at this width in CSS pixels instead of the person's current window. They can resize, so check a narrower and a wider frame if your composition might not survive one." },
                "height": { "type": "integer", "description": "Optional: render at this height in CSS pixels instead of the person's current window." },
            },
            "required": ["ref"],
        }),
    )
}

pub(crate) fn tools_for_role(role: Option<&str>) -> Vec<Value> {
    match role {
        Some("worker") => vec![
            send_message_tool(),
            review_view_tool(),
            tool(
                "look",
                "See the user's screen right now — returns a screenshot of the main display, plus \
                 its pixel size and the frontmost app. Use it to find where things are before you \
                 `act`, and again after acting to confirm what changed. The positions you pass to \
                 `act` are fractions of THIS image.",
                json!({ "type": "object", "properties": {} }),
            ),
            tool(
                "act",
                "Operate the user's screen like a human would: move, click, type, or press keys. \
                 Positions are normalized fractions of the screen read off the latest `look` — `x` \
                 is 0.0 (left) to 1.0 (right), `y` is 0.0 (top) to 1.0 (bottom). After you act, call \
                 `look` again to check it worked.",
                json!({
                    "type": "object",
                    "properties": {
                        "action": { "type": "string", "enum": ["click", "double_click", "right_click", "move", "drag", "type", "press"], "description": "What to do." },
                        "x": { "type": "number", "description": "Target x as a 0..1 fraction of screen width. For click/double_click/right_click/move, and the start of a drag." },
                        "y": { "type": "number", "description": "Target y as a 0..1 fraction of screen height." },
                        "x2": { "type": "number", "description": "Drag end x (0..1), for action=drag." },
                        "y2": { "type": "number", "description": "Drag end y (0..1), for action=drag." },
                        "text": { "type": "string", "description": "Text to type, for action=type (handles non-ASCII like a song title)." },
                        "key": { "type": "string", "description": "Key for action=press: return, tab, space, escape, delete, up, down, left, right, or a single character. For a chord like ⌘A use key=a with mods=[command]." },
                        "mods": { "type": "array", "items": { "type": "string", "enum": ["command", "shift", "option", "control"] }, "description": "Modifier keys held during a press." },
                    },
                    "required": ["action"],
                }),
            ),
            video_text_to_text_tool(),
        ]
        .into_iter()
        // Generation belongs to the rung that does the job. Reaction must stay a
        // voice (its surface is the one hard rail), Deliberation reads, Cognition
        // dispatches — a worker is the only rung that produces artifacts.
        .chain(generation_tools())
        .collect(),
        // The reflection ("sleep") surface: a voice-less session that consolidates
        // the one raw frontier into derived memory.
        Some("reflection") => vec![
            send_message_tool(),
            create_worker_tool(),
            session_status_tool(),
            session_messages_tool(),
            tool(
                "record_episode",
                "File one coherent event as an episode. You are shown the still-unconsolidated \
                 signals as one numbered list, oldest first. `count` is how many signals from the TOP \
                 of the remaining list this episode covers. Work front to back — each call consumes \
                 that many signals, so the next `count` starts after them. STOP early (just don't cover \
                 the last few) when the most recent signals are an event \
                 still in progress; they'll come back next time. `gist` is the consolidated event in your own \
                 prose. `title` is a short handle for this event (a few words) — it becomes the episode's \
                 directory name, so make it specific and human-readable (e.g. \"Lunch plan with Alice\", \
                 \"Kyoto flights booked\"). `subjects` are the `dimension/subject` refs this episode is about \
                 (e.g. `people/alice`, `projects/kyoto-trip`) — list every subject you'll want to update a \
                 facet for. The call returns the episode's ref; cite it when you update a facet.",
                json!({
                    "type": "object",
                    "properties": {
                        "count": { "type": "integer", "minimum": 1, "description": "How many signals from the top of the remaining unconsolidated list this episode covers." },
                        "title": { "type": "string", "description": "A short, specific handle for this event (a few words); becomes the episode's directory name, e.g. \"Lunch plan with Alice\"." },
                        "gist": { "type": "string", "description": "The consolidated event, in prose — what happened, what mattered." },
                        "subjects": { "type": "array", "items": { "type": "string" }, "description": "The dimension/subject refs this episode touches, e.g. [\"people/alice\", \"projects/kyoto-trip\"]." },
                    },
                    "required": ["count", "title", "gist"],
                }),
            ),
            tool(
                "read_facet",
                "Read your current understanding of one subject before you rewrite it, so you fold new \
                 episodes into what you already know instead of starting blank. Returns the facet's \
                 current text, or a note that none exists yet.",
                json!({
                    "type": "object",
                    "properties": {
                        "dimension": { "type": "string", "description": "The subject's dimension, e.g. people, locations, projects, culture." },
                        "subject": { "type": "string", "description": "The subject's name, e.g. alice, kyoto-trip." },
                    },
                    "required": ["dimension", "subject"],
                }),
            ),
            tool(
                "update_facet",
                "Write your whole current understanding of one subject — regenerate the file, don't patch \
                 it: pass the complete text (old understanding folded together with the new), not just a \
                 delta. Every claim should cite the episode(s) it came from by their refs (the values \
                 record_episode returned). Dimensions are open-ended; reuse an existing dimension/subject \
                 when one fits rather than coining a near-duplicate.",
                json!({
                    "type": "object",
                    "properties": {
                        "dimension": { "type": "string", "description": "The subject's dimension, e.g. people, locations, projects, culture." },
                        "subject": { "type": "string", "description": "The subject's name, e.g. alice, kyoto-trip." },
                        "content": { "type": "string", "description": "The full regenerated understanding (markdown), every claim citing its source episode refs." },
                    },
                    "required": ["dimension", "subject", "content"],
                }),
            ),
            tool(
                "name_person",
                "Attach a name to a person you've recognized. Faces and voices are clustered \
                 automatically — a face shows as `⟨faces: <id>⟩`, a speaker as `⟨voice: <id>⟩`, \
                 where an opaque id like `ff32ce3w` is someone not yet named. When a signal tells \
                 you who an id is (e.g. the person says their name, or someone introduces them), \
                 call this with `id` = that id and `name` = the name (the `people/<name>` ref you'd \
                 use for their facet). It renames the whole cluster from the id to the name, so you \
                 recognize them by name next time. If the name already exists, the two are merged.",
                json!({
                    "type": "object",
                    "properties": {
                        "id": { "type": "string", "description": "The cluster's current key — a `⟨faces: …⟩` or `⟨voice: …⟩` id (e.g. ff32ce3w), or an existing name to re-key." },
                        "name": { "type": "string", "description": "The person's name to key them under (e.g. 赵力, alice)." },
                    },
                    "required": ["id", "name"],
                }),
            ),
            tool(
                "merge_people",
                "Collapse two clusters that are the same person into one — when you realize a face \
                 or voice id (or a name) actually refers to someone you already model, including \
                 across senses (a `⟨voice: …⟩` id and a `⟨faces: …⟩` id that are one source). Folds \
                 `from`'s face/voice gallery into `into` and drops `from`.",
                json!({
                    "type": "object",
                    "properties": {
                        "from": { "type": "string", "description": "The duplicate cluster's key (an id or name) to fold away." },
                        "into": { "type": "string", "description": "The cluster's key (an id or name) to keep." },
                    },
                    "required": ["from", "into"],
                }),
            ),
            tool(
                "keep_and_fade",
                "Let a cold day's media fade to the text, keeping only the moments worth keeping \
                 vivid. Use it on a day from the old-store list you're shown — one genuinely old and \
                 settled, heaviest first — when the raw bytes are vividness the words have outlived. \
                 `channel` is `audio` or `vision`, `date` the `YYYY-MM-DD` day. `keep` is \
                 the spans to preserve, each `{start, end}` in RFC3339 — a vision keepsake is a still at \
                 `start`, an audio keepsake the clip `[start, end)`. Keep almost nothing: a frame or a few \
                 seconds, often none — pass `keep: []` to fade straight to text (which always remains). Keep \
                 only what the transcript can't carry (a face, a place, the sound of a voice), never someone \
                 merely talking. You can only fade a day already behind your consolidation; the tool \
                 refuses the rest.",
                json!({
                    "type": "object",
                    "properties": {
                        "channel": { "type": "string", "enum": ["audio", "vision"], "description": "Which sense's media to fade for this day." },
                        "date": { "type": "string", "description": "The day to fade, YYYY-MM-DD (UTC), from the old-store list." },
                        "keep": {
                            "type": "array",
                            "description": "Spans to keep vivid; omit or [] to fade straight to text. Each is one keepsake.",
                            "items": {
                                "type": "object",
                                "properties": {
                                    "start": { "type": "string", "description": "Span start, RFC3339 (the instant, for a vision still)." },
                                    "end": { "type": "string", "description": "Span end, RFC3339 (equal to start for a still; later for an audio clip)." },
                                },
                                "required": ["start", "end"],
                            },
                        },
                    },
                    "required": ["channel", "date"],
                }),
            ),
            tool(
                "update_proactivity",
                "Rewrite the agent's standing read on speaking up unprompted — the whole \
                 `proactivity.md`, regenerated, not patched. Reach for it when an unprompted word of the \
                 agent's landed (or fell flat) in the signals you just read: the agent spoke first, no one \
                 asked, and how that was met is what you're folding in. Pass the COMPLETE file: a short \
                 line per subject with where it now stands — `welcomed`, `tolerated`, `unproven`, or \
                 `muted` — and a few words of why, grounded in what actually happened. Be quick to pull a \
                 subject back on a brush-off or silence, slow to widen one on a single warm reception. \
                 Keep it short and scannable — the agent reads this before every proactive word.",
                json!({
                    "type": "object",
                    "properties": {
                        "content": { "type": "string", "description": "The full regenerated proactivity.md (markdown): a line per subject with its standing (welcomed/tolerated/unproven/muted) and a brief, evidence-based why." },
                    },
                    "required": ["content"],
                }),
            ),
            image_text_to_text_tool(),
        ],
        // The reaction is the fast conversational voice: it speaks via plain message
        // text (not a `say` tool) and gets exactly one expression tool — `show` —
        // to put a view a worker already built on screen. Nothing else; the heavy work
        // is delegated to workers in code, not via a tool.
        // The shared brain. It delegates rather than does, so its surface is the
        // switchboard and nothing else: hand work out, ask after it, read what came back.
        //
        // The ledger it owns needs no tool — a task is a plain facet on disk, and it has
        // the adapter's own Read/Write. That is why "sole writer of the ledger" is a
        // matter of which rung is *told* to write it, and why that instruction moving out
        // of `deliberation.md` and into `cognition.md` is the whole of the handover.
        //
        // No `say`, no `show`: it proposes, Reaction voices. Enforced three ways
        // that agree — absent here, refused at dispatch above, and its sink carries no
        // sequencer to express through.
        Some("cognition") => vec![
            send_message_tool(),
            create_worker_tool(),
            session_status_tool(),
            session_messages_tool(),
        ],
        // **Deliberation** — the conversation's reading and thinking. One tool, which is the
        // whole shape of the rung: it hands work *up*, and everything else it does it
        // does with the adapter's own Read/Write/fetch (`docs/arch/foundation.md`:
        // "SendMessage · reads · write **only** its conversation's brief").
        //
        // No `session_status`/`session_messages`: those belong to owners, and
        // Deliberation owns nothing — it creates no workers, and the hand-up to
        // Cognition is one-way by design (invariant 3: a conversation that waited on the one
        // global session would serialize every other conversation). The answer comes back as
        // mail, not as something to poll for.
        //
        // No `look`/`act`/`video-text-to-text` either, and that is the correction this arm exists
        // for. Until now a deliberation session was **opened as `SessionRole::Worker`**,
        // so it got the effector surface and its `X-HI-Role` said `worker` — a rung
        // wearing another rung's clothes. Its built-ins stay on: `foundation.md` is
        // explicit that only Reaction's surface is a rail, and this one is sized for
        // context.
        Some("deliberation") => vec![send_message_tool()],
        Some("reaction") => vec![say_tool(), show_tool(), send_message_tool()],
        // **Nothing.** Every role hi-agent opens is named above, so reaching here means
        // an unheadered or unknown session, and handing one an arbitrary toolset is how
        // the previous occupant of this arm survived: it held the legacy agentic
        // reaction's kit — `say`, `show`, `record_reflex`, and the two understanding tools —
        // long after no live role mapped to it, and read as a live surface in every
        // review. It was also a live hazard, not just clutter: `SessionRole::Deliberation`
        // stringifies to `deliberation`, which had no arm until the one above, so the
        // moment anything constructed that role it would have landed *here* and been
        // handed `say` (refused at dispatch) with no `send_message` at all.
        //
        // One tool lost its only declaration with this and was already unreachable:
        // `record_reflex`, which still has **no live role** — the recognizer and the
        // invoke route are real, so the reflex store can be read and fired but never
        // written. That is an open decision, not an oversight: it needs a rung or it
        // needs deleting, and it is now visibly nobody's rather than sitting in an arm
        // that looked live. (`alarm` was the other, and it is gone outright — see
        // `docs/arch/core.md` on the clock we declined.)
        _ => vec![],
    }
}

fn tool(name: &str, description: &str, input_schema: Value) -> Value {
    json!({ "name": name, "description": description, "inputSchema": input_schema })
}

// ---------------------------------------------------------------------------
// Modality tools
//
// Named for their Hugging Face task rather than for a verb — `image-text-to-text`
// where this was once `see`. The task name states the *signature* (what goes in,
// what comes out) where a verb states only an intent, and it is the same string the
// provider model cards use, so a tool, its capability module and its config key all
// carry one name. The trade: the name no longer reads as an instruction, so each
// `description` has to open with the action.
//
// Six tasks, one axis that matters:
//
//   understanding  `image-text-to-text`, `video-text-to-text`  → goes through
//       [`bundle::Bundle`]: raw pixels when the model takes them natively, the
//       vision capability's text when it doesn't.
//   generation     `text-to-image`, `image-to-image`,
//                  `text-to-video`, `image-to-video`           → never touches the
//       bundle. No model reached through the agent wire emits pixels, so these are
//       always a provider call.
//
// The four generation tools are declared and return [`not_implemented`]. The surface
// is settled here; the wiring waits on one unmade decision — where a generated
// artifact lands and what ref it gets — and minting that ref format twice is exactly
// the mistake this naming pass exists to stop.
// ---------------------------------------------------------------------------

/// `image-text-to-text` — an image plus an instruction in, text out.
fn image_text_to_text_tool() -> Value {
    tool(
        "image-text-to-text",
        "Look at a still image and answer about it — a photo the person sent, a screenshot they \
         handed over, or a frame held up to the camera. It reaches you as a signal carrying an \
         `⟨ref: …⟩`; pass that `ref`, and optionally what you want to know. Reach for it the moment \
         seeing the picture beats guessing: read a label/menu/handwriting, identify a thing, check \
         what's on a screen they photographed.",
        json!({
            "type": "object",
            "properties": {
                "ref": { "type": "string", "description": "The ⟨ref: …⟩ carried by the image's signal, e.g. 2026-06-25/14/23-07.jpg." },
                "prompt": { "type": "string", "description": "Optional: what you want to know about the image (a question or focus). Omit to just look." },
            },
            "required": ["ref"],
        }),
    )
}

/// `video-text-to-text` — a span of live camera plus an instruction in, text out.
/// Always polyfilled: no model behind the adapter takes video, so the clip is
/// understood by the vision capability and the text handed back.
fn video_text_to_text_tool() -> Value {
    tool(
        "video-text-to-text",
        "Watch a few seconds of the live camera and tell what happened — for when motion or a \
         sequence matters, not a single frame (someone's action, a gesture, \"did you see that?\"). \
         It reads the camera streaming right now; say how far back with `span` (e.g. \"last 20s\"), or \
         omit it for the most recent stretch. Carry seconds, not minutes. Optionally say what to look \
         for with `prompt`.",
        json!({
            "type": "object",
            "properties": {
                "span": { "type": "string", "description": "How far back to look, e.g. \"last 20s\". Omit for the most recent stretch." },
                "prompt": { "type": "string", "description": "Optional: what to look for or assess (e.g. \"what's wrong with my serve?\")." },
            },
        }),
    )
}

/// `text-to-image` — a prompt in, a new image out. Backed by
/// [`crate::body::capabilities::image_gen`], which is configured but has no caller.
fn text_to_image_tool() -> Value {
    tool(
        "text-to-image",
        "Draw a new image from a description. Say what it should show; `size` and `seed` are \
         optional vendor knobs (a fixed seed makes a run repeatable).",
        json!({
            "type": "object",
            "properties": {
                "prompt": { "type": "string", "description": "What the image should show." },
                "size": { "type": "string", "description": "Optional, vendor-specific: e.g. \"1024x1024\", \"2K\", \"adaptive\"." },
                "seed": { "type": "integer", "description": "Optional: fix the seed to make the result repeatable." },
            },
            "required": ["prompt"],
        }),
    )
}

/// `image-to-image` — an existing image plus an instruction in, a new image out. No
/// capability behind it yet: [`crate::body::capabilities::image_gen::ImageRequest`]
/// carries no input image, so editing is currently unrepresentable end to end.
fn image_to_image_tool() -> Value {
    tool(
        "image-to-image",
        "Edit an existing image — say what to change and it returns a new image, leaving the \
         original untouched. Pass the `⟨ref: …⟩` of the image to work from.",
        json!({
            "type": "object",
            "properties": {
                "ref": { "type": "string", "description": "The ⟨ref: …⟩ of the image to edit, e.g. 2026-06-25/14/23-07.jpg." },
                "prompt": { "type": "string", "description": "What to change (e.g. \"make the sky overcast\", \"remove the car\")." },
                "size": { "type": "string", "description": "Optional, vendor-specific output size." },
                "seed": { "type": "integer", "description": "Optional: fix the seed to make the result repeatable." },
            },
            "required": ["ref", "prompt"],
        }),
    )
}

/// `text-to-video` — a prompt in, a clip out. Backed by
/// [`crate::body::capabilities::video_gen`], which is configured but has no caller.
fn text_to_video_tool() -> Value {
    tool(
        "text-to-video",
        "Generate a short video clip from a description. Generation is slow — this enqueues the \
         work and the clip arrives when it's done, so don't wait on it inline.",
        json!({
            "type": "object",
            "properties": {
                "prompt": { "type": "string", "description": "What the clip should show." },
                "duration": { "type": "integer", "description": "Optional: clip length in seconds." },
                "ratio": { "type": "string", "description": "Optional: aspect ratio, e.g. \"16:9\", \"9:16\", \"1:1\"." },
                "resolution": { "type": "string", "description": "Optional: e.g. \"480p\", \"720p\", \"1080p\"." },
                "seed": { "type": "integer", "description": "Optional: fix the seed to make the result repeatable." },
            },
            "required": ["prompt"],
        }),
    )
}

/// `image-to-video` — an image as first frame plus an optional prompt in, a clip out.
/// The one generation task the capability layer already models:
/// [`crate::body::capabilities::video_gen::VideoRequest::first_frame`].
fn image_to_video_tool() -> Value {
    tool(
        "image-to-video",
        "Animate an existing still — it becomes the first frame of a short clip. Pass the \
         `⟨ref: …⟩` of the image, and optionally say how it should move. Slow, like any \
         generation: it enqueues and the clip arrives later.",
        json!({
            "type": "object",
            "properties": {
                "ref": { "type": "string", "description": "The ⟨ref: …⟩ of the still to animate from, e.g. 2026-06-25/14/23-07.jpg." },
                "prompt": { "type": "string", "description": "Optional: how it should move or what should happen." },
                "duration": { "type": "integer", "description": "Optional: clip length in seconds." },
                "ratio": { "type": "string", "description": "Optional: aspect ratio, e.g. \"16:9\", \"9:16\", \"1:1\"." },
                "resolution": { "type": "string", "description": "Optional: e.g. \"480p\", \"720p\", \"1080p\"." },
                "seed": { "type": "integer", "description": "Optional: fix the seed to make the result repeatable." },
            },
            "required": ["ref"],
        }),
    )
}

/// The four generation tasks, as one surface. Declared together because they share
/// the blocker (no artifact home, no ref) and will be wired together.
fn generation_tools() -> Vec<Value> {
    vec![text_to_image_tool(), image_to_image_tool(), text_to_video_tool(), image_to_video_tool()]
}

/// The `show` tool — put a view on the screen. The reaction's one expression
/// tool beyond speech: it shows a view a worker already built (by `ref`), or a
/// trivial inline one. Shared by the reaction surface and the legacy fallback.
fn show_tool() -> Value {
    tool(
        "show",
        "Put a view on the screen. Normally you show a view a builder made for you: \
         delegate the build, then pass the `ref` it reported back (like `project/view`) here. \
         Interleave show and say calls in the order you want them experienced (say, \
         then show) so each view lands as you speak to it. \
         The screen holds ONE view at a time, filling it edge to edge. Showing is \
         therefore how you *change* the screen, not how you add to it: a show under a \
         new id replaces whatever was up, so walking someone through a sequence is just \
         show, say, show, say — you never need to dismiss between beats, and there is no \
         way to end up with two things piled on screen. Reuse an `id` with op=replace to \
         evolve one view in place (the slot is kept, so a motion-tagged element animates \
         rather than blinking); op=dismiss clears the screen back to the empty room, which \
         is what you want when the topic is over and nothing replaces it. \
         The screen is persistent state: what you've shown stays up across page refreshes, \
         other devices in the conversation, even restarts, until something replaces it or you \
         dismiss it. What is up right now is listed under `## On screen now` in your \
         context — trust that list, don't guess. If it says the room is clear, there is \
         nothing to dismiss; don't fire dismisses at remembered ids. \
         For a trivial one-off you may pass raw `source` JSX instead of a ref.",
        json!({
            "type": "object",
            "properties": {
                "op": { "type": "string", "enum": ["show", "replace", "dismiss"], "description": "show puts this view up, replacing whatever was on screen; replace swaps the same id in place, keeping the slot so motion animates; dismiss clears the screen." },
                "id": { "type": "string", "description": "A stable name for this on-screen slot, so replace/dismiss can target it. Omit to auto-generate." },
                "ref": { "type": "string", "description": "A view ref a builder reported (e.g. `project/view`) — the usual way to show a built view. Omit for dismiss." },
                "source": { "type": "string", "description": "Raw JSX (default-exported component) for a trivial inline view, when not using a ref. Omit for dismiss." },
            },
            "required": ["op"],
        }),
    )
}

/// Handle one parsed JSON-RPC message. `role` and `session_id` come from the
/// request headers; `registry` routes loop-owned tool calls by role.
pub async fn handle(
    registry: &ToolRegistry,
    data_dir: &std::path::Path,
    video_partial: &Mutex<Option<PartialMinute>>,
    observatory: &Observatory,
    role: Option<&str>,
    session_id: Option<u64>,
    msg: &Value,
) -> McpReply {
    let method = msg.get("method").and_then(Value::as_str).unwrap_or_default();
    let id = msg.get("id").cloned();

    // No id ⇒ a notification (e.g. notifications/initialized) ⇒ just 202.
    let Some(id) = id else {
        return McpReply::Accepted;
    };

    match method {
        "initialize" => {
            let requested = msg
                .get("params")
                .and_then(|p| p.get("protocolVersion"))
                .and_then(Value::as_str)
                .unwrap_or(PROTOCOL_VERSION);
            McpReply::Json(result(
                id,
                json!({
                    "protocolVersion": requested,
                    "capabilities": { "tools": {} },
                    "serverInfo": { "name": "hi-agent", "version": env!("CARGO_PKG_VERSION") },
                }),
            ))
        }
        "tools/list" => McpReply::Json(result(id, json!({ "tools": tools_for_role(role) }))),
        "tools/call" => {
            let params = msg.get("params").cloned().unwrap_or_else(|| json!({}));
            let name = params.get("name").and_then(Value::as_str).unwrap_or_default();
            let args = params.get("arguments").cloned().unwrap_or_else(|| json!({}));
            McpReply::Json(result(
                id,
                dispatch_tool(registry, data_dir, video_partial, observatory, session_id, role, name, &args).await,
            ))
        }
        // ping is a no-op request the client may send.
        "ping" => McpReply::Json(result(id, json!({}))),
        other => McpReply::Json(error(id, -32601, &format!("method not found: {other}"))),
    }
}

/// Run one tool call, returning the MCP `tools/call` result shape (a content list
/// with an `isError` flag). Tools are fire-and-forget: we forward the call to the
/// owning loop (for side-effects or sequenced output) and ack
/// immediately, never blocking on playback or on the worker a delegate spawns.
async fn dispatch_tool(
    registry: &ToolRegistry,
    data_dir: &std::path::Path,
    video_partial: &Mutex<Option<PartialMinute>>,
    observatory: &Observatory,
    session_id: Option<u64>,
    role: Option<&str>,
    name: &str,
    args: &Value,
) -> Value {
    // Expression tools belong to the reaction alone — it is the single
    // guideline-carrying voice, so everything the person sees or hears goes through
    // its `reaction.md` generation. A worker or reflection session must never speak
    // or take the screen even if its model emits the call (these aren't in its
    // advertised surface); enforce that structurally here, not just via the tool list.
    if matches!(name, "say" | "show") && role != Some("reaction") {
        return tool_error(&format!(
            "`{name}` is reaction-only; role `{}` may not speak or show",
            role.unwrap_or("<none>")
        ));
    }

    // Reflection tools are pure derived-memory IO over `data_dir`; they don't touch
    // a loop sink, so handle them before the sink lookup.
    match name {
        "record_episode" => return reflection_record_episode(data_dir, args).await,
        "read_facet" => return reflection_read_facet(data_dir, args).await,
        "update_facet" => return reflection_update_facet(data_dir, args).await,
        "update_proactivity" => return reflection_update_proactivity(data_dir, args).await,
        "name_person" => return reflection_name_person(data_dir, args).await,
        "merge_people" => return reflection_merge_people(data_dir, args).await,
        "keep_and_fade" => return reflection_keep_and_fade(data_dir, args).await,
        // Reachable by name only — `record_reflex` is advertised to no role, because
        // the reflex rung is **deferred** (see `body::reflex`). Kept dispatchable rather
        // than deleted so the authoring half is one arm entry away when it gets a rung,
        // and so a session that somehow names it gets the real behaviour instead of
        // "unknown tool". Do not re-advertise it without taking that decision.
        "record_reflex" => return reflex_record(data_dir, args).await,
        "look" => return do_look().await,
        "act" => return do_act(args).await,
        "image-text-to-text" => return do_image_text_to_text(data_dir, args).await,
        "video-text-to-text" => {
            return do_video_text_to_text(data_dir, video_partial, args).await;
        }
        // Declared, advertised, and not yet wired. They fail loudly rather than
        // being absent, because an absent tool reads to the model as "this agent
        // cannot" — a different claim, and the wrong one. Each names the seam that
        // is actually missing and reports live provider state, so the message is
        // checked rather than guessed.
        "text-to-image" => {
            return not_implemented(
                "text-to-image",
                "the vendor call `image_gen::text_to_image` exists; what is missing is the handler \
                 that persists the result and mints a ref for it",
                image_gen::available(),
            );
        }
        "image-to-image" => {
            return not_implemented(
                "image-to-image",
                "`image_gen::image_to_image` is declared but has no vendor arm, and no handler \
                 persists a result",
                image_gen::available(),
            );
        }
        "text-to-video" => {
            return not_implemented(
                "text-to-video",
                "the vendor call `video_gen::text_to_video` exists; what is missing is the handler \
                 that polls the task to a terminal state and lands the clip",
                video_gen::available(),
            );
        }
        "image-to-video" => {
            return not_implemented(
                "image-to-video",
                "the vendor call `video_gen::image_to_video` exists; what is missing is resolving \
                 the input ref to bytes, plus the handler that polls and lands the clip",
                video_gen::available(),
            );
        }
        "review_view" => return do_review_view(data_dir, args).await,
        _ => {}
    }

    let arg_str =
        |key: &str| args.get(key).and_then(Value::as_str).unwrap_or_default().to_string();
    let arg_opt = |key: &str| args.get(key).and_then(Value::as_str).map(str::to_owned);

    // The switchboard calls, handled **before** any conversation is looked up.
    //
    // They reach the process-wide session registry and touch no conversation at all, so
    // requiring a live reaction loop to make one was a category error — and a load-bearing
    // one: `create_worker` belongs to the standing rungs, and Reflection runs under a
    // sentinel conversation that has no loop, so the one rung holding the tool was the one rung
    // that could never call it. Same for the one verb: a standing agent could be sent
    // to, but could not send.
    match name {
        "send_message" => {
            let Some(from) = session_id else {
                return tool_error("send_message needs a session identity; this session has none");
            };
            let to = arg_str("to");
            let message = arg_str("message");
            if to.trim().is_empty() || message.trim().is_empty() {
                return tool_error("send_message requires `to` and a non-empty `message`");
            }
            let Ok(target) = to.trim().parse::<u64>() else {
                return tool_error(&format!(
                    "`{}` is not a session id. Addresses are numbers — a worker's comes \
                     back from `create_worker`, and everyone else you can reach is listed \
                     in your window with theirs.",
                    to.trim()
                ));
            };
            let delivery = registry::global().send(from, target, message.clone());

            // The edge, observed. Attributed to the **sender's** conversation, because that is
            // the one fact we hold at this point — the switchboard resolves the target
            // and does not report its conversation. For a standing rung the header carries a
            // sentinel, so pass `None` rather than let a placeholder become a
            // conversation in the mirror.
            observatory
                .record(
                    EventKind::MessageSent { from: Some(from), to: target, delivery, message },
                )
                .await;

            return match delivery {
                registry::Delivery::Delivered => tool_ok("delivered"),
                registry::Delivery::Unknown => tool_error(&format!(
                    "nothing live at `{}` — it may have finished. Nothing was delivered.",
                    to.trim()
                )),
                registry::Delivery::NotPermitted => tool_error(
                    "a working session reports to whoever asked for the work, and to nobody else",
                ),
            };
        }
        "session_status" => {
            let Some(id) = arg_str("id").trim().parse::<u64>().ok() else {
                return tool_error("session_status requires a numeric `id`");
            };
            let Some(st) = registry::global().status(id) else {
                return tool_error(&format!("no live session {id}"));
            };
            let state = if st.busy {
                "working right now"
            } else if st.queued {
                "idle, with mail waiting"
            } else {
                "idle"
            };
            return tool_ok(&format!(
                "session {} — {state}; {} turn(s) so far; on: {}",
                st.id, st.turns, st.task
            ));
        }
        "session_messages" => {
            let Some(id) = arg_str("id").trim().parse::<u64>().ok() else {
                return tool_error("session_messages requires a numeric `id`");
            };
            return match registry::global().messages(id) {
                Some(text) if !text.trim().is_empty() => tool_ok(&text),
                Some(_) => tool_ok("that session has not said anything yet"),
                None => tool_error(&format!("no live session {id}")),
            };
        }
        "create_worker" => {
            // Workers belong to the standing rungs — Cognition and Reflection, per
            // `docs/arch/foundation.md`. Reaction speaks and Deliberation reads; neither
            // dispatches, and a conversation-bound rung that could would be a second dispatcher
            // (`docs/arch/agents.md`: "one dispatcher is the point").
            //
            // Structural, not just absent from the advertised surface — the same reason
            // `say`/`show` are checked above. Until now this was enforced only by
            // accident: Reaction had no `X-HI-Session-Id`, so the identity check below
            // rejected it. That fence is gone as of this commit, so the real one goes in.
            if !matches!(role, Some("reflection") | Some("cognition")) {
                return tool_error(&format!(
                    "`create_worker` belongs to the standing rungs; role `{}` may not dispatch work",
                    role.unwrap_or("<none>")
                ));
            }
            let Some(owner) = session_id else {
                return tool_error("create_worker needs a session identity; this session has none");
            };
            let task = arg_str("task");
            if task.trim().is_empty() {
                return tool_error("create_worker requires a non-empty `task`");
            }
            // Absent means `general`, which is the right answer for most work. A name we
            // do not know is an **error**, not a silent fall back to general: a mistyped
            // `view-buidler` that quietly becomes a general session is a worker that will
            // not do the job it was made for, and nothing anywhere says so. The schema
            // constrains this too; this is the half that still holds when it doesn't.
            let kind = match args.get("type").and_then(|v| v.as_str()) {
                None => WorkerType::default(),
                Some(name) => match WorkerType::parse(name) {
                    Some(k) => k,
                    None => {
                        let valid: Vec<_> = WorkerType::ALL.iter().map(|t| t.as_str()).collect();
                        return tool_error(&format!(
                            "unknown worker type `{name}` — one of: {}",
                            valid.join(", ")
                        ));
                    }
                },
            };
            // The id is minted **here**, before the session exists, and handed back in
            // this reply — the contract is `CreateWorker → a session id`, and a caller
            // that cannot name what it made cannot brief it, ask after it, or read it.
            // Minting early is the same trick the whole session layer already uses: the
            // tool surface identifies its caller by a header, so an id has to exist
            // before the protocol assigns one.
            let id = registry::mint();
            // A worker must run in the standing loop that created it. Role-specific
            // slots keep Cognition and Reflection from replacing each other's route.
            let owner_role = ToolOwner::from_role(role).expect("role guard above");
            let Some(sink) = registry.get(owner_role).await else {
                return tool_error(
                    "the owning loop is not up, so there is nowhere to run a worker",
                );
            };
            return match sink
                .send(LoopControl::CreateWorker { id, task, kind, owner: Some(owner) })
                .await
            {
                Ok(()) => tool_ok(&format!(
                    "session {id} starting; brief it with send_message, check it with \
                     session_status"
                )),
                Err(err) => tool_error(&err.to_string()),
            };
        }
        _ => {}
    }

    let Some(owner) = ToolOwner::from_role(role) else {
        return tool_error("this role has no loop-owned tools");
    };
    let Some(sink) = registry.get(owner).await else {
        return tool_error("the owning loop is not up");
    };

    let outcome = match name {
        "say" => {
            let text = arg_str("text");
            if text.trim().is_empty() {
                return tool_error("say requires non-empty `text`");
            }
            // The ack is what actually happened on each channel, not a constant: the
            // tool's whole justification is that speech is answerable, and an answer
            // that always reads "spoken" answers nothing.
            sink.say(text).await.map(crate::body::reaction::Spoken::ack)
        }
        "show" => {
            let op = args.get("op").and_then(Value::as_str).unwrap_or("show").to_string();
            // A view is normally shown by ref (one a worker built); resolve it to
            // source HERE, server-side, so the JSX never enters the mind's context.
            // Inline `source` stays as a trivial-one-off escape hatch. The ref may
            // carry a `.geom.json` sidecar — what the builder declared about it.
            // There is no placement to override here any more: views are full-bleed,
            // one at a time, so the mind decides *what* is on screen and never where.
            let (source, traits) = match arg_opt("ref") {
                Some(r) if !r.trim().is_empty() => match resolve_view_ref(data_dir, &r).await {
                    Ok(resolved) => resolved,
                    Err(err) => return tool_error(&format!("show ref `{r}`: {err}")),
                },
                _ => (arg_str("source"), None),
            };
            sink.show(arg_opt("id"), op, source, traits).await.map(|()| "shown")
        }
        other => return tool_error(&format!("unknown tool: {other}")),
    };

    match outcome {
        Ok(ack) => tool_ok(ack),
        Err(err) => tool_error(&err.to_string()),
    }
}

/// `look`: capture the screen so the calling session can see where to act. Returns
/// a text hint (size + frontmost app) and the screenshot as an image content block,
/// which codex forwards to the model as an `input_image`. Errors when capture
/// is unavailable (non-macOS, or Screen Recording not granted).
async fn do_look() -> Value {
    let snap = match crate::body::capabilities::desktop_context::capture().await {
        Ok(s) => s,
        Err(e) => return tool_error(&format!("screen capture not available here: {e}")),
    };
    let Some(png) = snap.screenshot_png else {
        return tool_error("no screenshot — grant Screen Recording to the host app");
    };
    let mut hint = match png_dimensions(&png) {
        Some((w, h)) => format!("screenshot of the main display, {w}x{h} px"),
        None => "screenshot of the main display".to_string(),
    };
    if let Some(app) = &snap.frontmost_app {
        hint.push_str(&format!("; frontmost app: {app}"));
    }
    if let Some(title) = &snap.frontmost_window_title {
        hint.push_str(&format!("; front window: {title}"));
    }
    let b64 = base64::engine::general_purpose::STANDARD.encode(&png);
    json!({
        "content": [
            { "type": "text", "text": hint },
            { "type": "image", "data": b64, "mimeType": "image/png" },
        ],
        "isError": false,
    })
}

/// `review_view`: compile the saved view, render it in a real browser, and answer with
/// the verdict, the page's problems, and the screenshot.
///
/// The three steps are the same ones the build path takes, in the same order, which is
/// why this is a thin call rather than a second renderer: resolve the ref to source
/// (the compiler's existing job), compile it to a served module URL, then hand that to
/// [`view_render::render`], which owns the browser, the viewport policy and the blank
/// detection.
///
/// **The review frame IS the stage frame** — full-bleed, the only frame there is — so
/// a review renders the thing exactly the way `show` will put it up. This used to be a
/// negotiation between the caller's override and the sidecar's declared region, and
/// getting it wrong failed a view for a defect the review itself introduced.
async fn do_review_view(data_dir: &std::path::Path, args: &Value) -> Value {
    let view_ref = args.get("ref").and_then(Value::as_str).unwrap_or_default().trim().to_string();
    if view_ref.is_empty() {
        return tool_error("review_view requires a `ref`");
    }
    let (source, traits) = match resolve_view_ref(data_dir, &view_ref).await {
        Ok(v) => v,
        Err(e) => return tool_error(&e),
    };
    // Published at startup. Absent means the process never stood the view path up —
    // a condition to report, not to panic on, and one a unit test hits by construction.
    let Some(ctx) = crate::mind::views::render_context() else {
        return tool_error("the view renderer is not available in this process");
    };
    let module_url = match ctx.compiler.compile(&source).await {
        Ok(u) => u,
        Err(e) => return tool_error(&format!("the view did not compile: {e}")),
    };

    // Every view renders full-bleed, so there is no placement to resolve or override:
    // the review page shows the view at exactly the frame it will occupy on the stage.
    // That equivalence is the whole point of the review — it used to be conditional on
    // the reviewer and the sidecar agreeing about a region.
    //
    // Size is the half of that equivalence placement never covered. `RenderRequest::new`
    // takes the frame the window last reported, so "exactly the frame" now means the
    // person's actual window rather than a constant that matched none of them.
    let mut req = view_render::RenderRequest::new(&ctx.base_url, module_url)
        .with_captions(traits.is_some_and(|t| t.owns_captions));

    // An explicit size is a deliberate second look at another frame, so it overrides one
    // axis at a time: asking for a narrower width alone should not also snap the height
    // back to a default the person's window never had.
    if let Some(w) = args.get("width").and_then(Value::as_u64) {
        req.viewport.width = w as u32;
    }
    if let Some(h) = args.get("height").and_then(Value::as_u64) {
        req.viewport.height = h as u32;
    }
    if !(320..=16_384).contains(&req.viewport.width)
        || !(320..=16_384).contains(&req.viewport.height)
    {
        return tool_error(
            "`width`/`height` are CSS pixels and must be between 320 and 16384 — a frame \
             outside that is not a window anyone is looking at",
        );
    }

    // **Both skins, unless the caller pinned one.** Theme is a live setting the person
    // controls (Settings ▸ General ▸ Theme), so "it rendered" is only true once it has
    // rendered the way they may actually be looking at it. A single-theme review cannot
    // see the defect this is here to catch: a colour that resolves in one skin and not
    // the other — a hardcoded ground under a `var(--fg)` that flips, or a token name
    // that was never defined, so its one-theme fallback always wins. Both bundled-view
    // contrast failures we shipped were invisible in a light-only render.
    // Language is opt-in, unlike the theme sweep: only the bundled system views carry
    // more than one language, so forcing a second render of every agent-authored view
    // would double the cost for nothing.
    req.lang = args.get("lang").and_then(Value::as_str).map(str::to_owned);

    let pinned = args.get("theme").and_then(Value::as_str).map(str::to_owned);
    let themes: Vec<String> = match pinned {
        Some(t) => vec![t],
        None => vec!["light".to_string(), "dark".to_string()],
    };

    let mut shots: Vec<(String, Vec<u8>)> = Vec::new();
    let mut failures: Vec<String> = Vec::new();
    for theme in &themes {
        req.theme = Some(theme.clone());
        let rendered = match view_render::render(&req).await {
            Ok(r) => r,
            Err(e) => return tool_error(&format!("could not render `{view_ref}` ({theme}): {e}")),
        };
        if let view_render::Verdict::Failed(why) = rendered.verdict() {
            failures.push(format!("{theme}: {why}"));
        }
        shots.push((theme.clone(), rendered.png));
    }

    // The verdict first, in words, because that is the answer. The pictures follow so
    // the reviewer can disagree with it — a view can render cleanly and still be bad,
    // and that judgment is the whole reason a session is doing this rather than a
    // pass/fail check in the build. Contrast is deliberately *not* scored here for the
    // same reason: the eye that catches "the names went invisible" is the one looking
    // at both frames, not a threshold.
    let summary = if !failures.is_empty() {
        format!("`{view_ref}` did not render properly — {}", failures.join("; "))
    } else if shots.len() > 1 {
        format!(
            "`{view_ref}` rendered in both skins. Nothing is broken — now judge whether it is \
             any good, and compare the two frames: anything that fades out, disappears or \
             turns unreadable in one of them is a colour that only works in the other."
        )
    } else {
        format!("`{view_ref}` rendered. Nothing is broken — now judge whether it is any good.")
    };

    let mut content = vec![json!({ "type": "text", "text": summary })];
    for (theme, png) in &shots {
        if shots.len() > 1 {
            content.push(json!({ "type": "text", "text": format!("— {theme} —") }));
        }
        content.push(json!({
            "type": "image",
            "data": base64::engine::general_purpose::STANDARD.encode(png),
            "mimeType": "image/png",
        }));
    }
    json!({ "content": content, "isError": false })
}

/// `act`: synthesize one input action on the host. Coordinates arrive as normalized
/// 0..1 fractions of the screen (what the model reasons about, looking at `look`'s
/// image) and are mapped to the main display's points here, so the pixel-vs-point
/// Retina detail never reaches the model.
async fn do_act(args: &Value) -> Value {
    use crate::body::capabilities::input::{self, Action, Point};
    let action = args.get("action").and_then(Value::as_str).unwrap_or_default();

    let act = match action {
        "type" => {
            let text = args.get("text").and_then(Value::as_str).unwrap_or_default();
            if text.is_empty() {
                return tool_error("act `type` requires non-empty `text`");
            }
            Action::Type(text.to_string())
        }
        "press" => {
            let Some(key) = parse_key(args.get("key").and_then(Value::as_str).unwrap_or_default())
            else {
                return tool_error(
                    "act `press` needs a valid `key`: return, tab, space, escape, delete, \
                     up/down/left/right, or a single character",
                );
            };
            Action::Press { key, mods: parse_mods(args.get("mods")) }
        }
        "click" | "double_click" | "right_click" | "move" | "drag" => {
            let (w, h) = match input::main_display_point_size() {
                Ok(s) => s,
                Err(e) => return tool_error(&format!("could not read display size: {e}")),
            };
            let pt = |xk: &str, yk: &str| -> Option<Point> {
                let x = args.get(xk).and_then(Value::as_f64)?;
                let y = args.get(yk).and_then(Value::as_f64)?;
                Some(Point { x: x.clamp(0.0, 1.0) * w, y: y.clamp(0.0, 1.0) * h })
            };
            let Some(from) = pt("x", "y") else {
                return tool_error("act requires `x` and `y` as 0..1 fractions of the screen");
            };
            match action {
                "click" => Action::Click(from),
                "double_click" => Action::DoubleClick(from),
                "right_click" => Action::RightClick(from),
                "move" => Action::MoveTo(from),
                "drag" => {
                    let Some(to) = pt("x2", "y2") else {
                        return tool_error("act `drag` requires `x2` and `y2` (the drag end, 0..1)");
                    };
                    Action::Drag { from, to }
                }
                _ => unreachable!(),
            }
        }
        other => return tool_error(&format!("unknown act action `{other}`")),
    };

    match input::perform(act).await {
        Ok(()) => tool_ok("acted"),
        Err(e) => tool_error(&e.to_string()),
    }
}

/// Read (width, height) from a PNG's IHDR header — big-endian, right after the
/// 8-byte signature. `None` if the bytes aren't a PNG we recognize.
fn png_dimensions(png: &[u8]) -> Option<(u32, u32)> {
    if png.len() < 24 || &png[12..16] != b"IHDR" {
        return None;
    }
    let w = u32::from_be_bytes(png[16..20].try_into().ok()?);
    let h = u32::from_be_bytes(png[20..24].try_into().ok()?);
    Some((w, h))
}

/// Map an `act` `key` string to a [`crate::body::capabilities::input::Key`]. Named keys
/// are case-insensitive; anything else is taken as a single character (so `a`, `/`,
/// `7` work). `None` for an empty or multi-character unknown name.
fn parse_key(s: &str) -> Option<crate::body::capabilities::input::Key> {
    use crate::body::capabilities::input::Key;
    Some(match s.to_ascii_lowercase().as_str() {
        "return" | "enter" => Key::Return,
        "tab" => Key::Tab,
        "space" => Key::Space,
        "escape" | "esc" => Key::Escape,
        "delete" | "backspace" => Key::Delete,
        "up" => Key::ArrowUp,
        "down" => Key::ArrowDown,
        "left" => Key::ArrowLeft,
        "right" => Key::ArrowRight,
        other => {
            let mut chars = other.chars();
            let c = chars.next()?;
            if chars.next().is_some() {
                return None;
            }
            Key::Char(c)
        }
    })
}

/// Map an `act` `mods` array to modifiers, accepting common aliases. Unknown
/// entries are dropped.
fn parse_mods(v: Option<&Value>) -> Vec<crate::body::capabilities::input::Modifier> {
    use crate::body::capabilities::input::Modifier;
    v.and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|m| {
                    Some(match m.as_str()?.to_ascii_lowercase().as_str() {
                        "command" | "cmd" | "meta" => Modifier::Command,
                        "shift" => Modifier::Shift,
                        "option" | "alt" => Modifier::Option,
                        "control" | "ctrl" => Modifier::Control,
                        _ => return None,
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// `record_episode`: file the first `count` unconsolidated signals as one episode
/// (see [`crate::mind::memory::episodes::record_episode`]).
/// Returns the episode ref for the session to cite when it updates a facet.
async fn reflection_record_episode(data_dir: &std::path::Path, args: &Value) -> Value {
    let Some(count) = args.get("count").and_then(Value::as_u64) else {
        return tool_error("record_episode requires an integer `count` >= 1");
    };
    let gist = args.get("gist").and_then(Value::as_str).unwrap_or_default();
    if gist.trim().is_empty() {
        return tool_error("record_episode requires a non-empty `gist`");
    }
    let title = args.get("title").and_then(Value::as_str).unwrap_or_default();
    if title.trim().is_empty() {
        return tool_error("record_episode requires a non-empty `title`");
    }
    let subjects: Vec<String> = args
        .get("subjects")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_owned)).collect())
        .unwrap_or_default();
    match crate::mind::memory::episodes::record_episode(data_dir, count as usize, title, gist, &subjects)
        .await
    {
        Ok(name) => tool_ok(&format!("recorded episode {name}")),
        Err(err) => tool_error(&err.to_string()),
    }
}

/// `read_facet`: return the current understanding of a subject, or a note that
/// none exists yet, so the session regenerates from the old rather than blank.
async fn reflection_read_facet(data_dir: &std::path::Path, args: &Value) -> Value {
    let dim = args.get("dimension").and_then(Value::as_str).unwrap_or_default();
    let subject = args.get("subject").and_then(Value::as_str).unwrap_or_default();
    if dim.trim().is_empty() || subject.trim().is_empty() {
        return tool_error("read_facet requires `dimension` and `subject`");
    }
    match crate::mind::memory::facets::read_facet(data_dir, dim, subject).await {
        Ok(Some(content)) => tool_ok(&content),
        Ok(None) => tool_ok("(no facet yet — this subject has no recorded understanding)"),
        Err(err) => tool_error(&err.to_string()),
    }
}

/// `update_facet`: write the whole regenerated understanding of a subject (see
/// [`crate::mind::memory::facets::update_facet`]). Returns the `<dim>/<subject>` ref.
async fn reflection_update_facet(data_dir: &std::path::Path, args: &Value) -> Value {
    let dim = args.get("dimension").and_then(Value::as_str).unwrap_or_default();
    let subject = args.get("subject").and_then(Value::as_str).unwrap_or_default();
    let content = args.get("content").and_then(Value::as_str).unwrap_or_default();
    if dim.trim().is_empty() || subject.trim().is_empty() {
        return tool_error("update_facet requires `dimension` and `subject`");
    }
    if content.trim().is_empty() {
        return tool_error("update_facet requires non-empty `content`");
    }
    match crate::mind::memory::facets::update_facet(data_dir, dim, subject, content).await {
        Ok(refname) => tool_ok(&format!("updated facet {refname}")),
        Err(err) => tool_error(&err.to_string()),
    }
}

/// `update_proactivity`: regenerate the whole `proactivity.md` — the learned
/// read on speaking up unprompted — from how the agent's own unprompted utterances
/// landed (see [`crate::mind::memory::proactivity::write`]). Whole-file, never a patch.
async fn reflection_update_proactivity(data_dir: &std::path::Path, args: &Value) -> Value {
    let content = args.get("content").and_then(Value::as_str).unwrap_or_default();
    if content.trim().is_empty() {
        return tool_error("update_proactivity requires non-empty `content`");
    }
    match crate::mind::memory::proactivity::write(data_dir, content).await {
        Ok(()) => tool_ok("updated proactivity.md"),
        Err(err) => tool_error(&err.to_string()),
    }
}

/// `record_reflex`: teach a quick-action reflex (see [`crate::body::reflex`]). Stores the
/// fill value and how to find its field so a later invoke types it with no model in
/// the loop. The value itself is never echoed back in the ack.
async fn reflex_record(data_dir: &std::path::Path, args: &Value) -> Value {
    let name = args.get("name").and_then(Value::as_str).unwrap_or_default();
    let value = args.get("value").and_then(Value::as_str).unwrap_or_default();
    let label_contains = args.get("label_contains").and_then(Value::as_str).unwrap_or_default();
    if name.trim().is_empty() {
        return tool_error("record_reflex requires a non-empty `name`");
    }
    if value.trim().is_empty() {
        return tool_error("record_reflex requires a non-empty `value`");
    }
    if label_contains.trim().is_empty() {
        return tool_error("record_reflex requires a non-empty `label_contains`");
    }
    let opt = |k: &str| {
        args.get(k)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
    };
    let id = crate::body::reflex::id_for(name);
    if id.is_empty() {
        return tool_error("record_reflex `name` must contain a usable character");
    }
    let reflex = crate::body::reflex::Reflex {
        id,
        name: name.to_string(),
        trigger: crate::body::reflex::Trigger {
            app: opt("app"),
            title_contains: opt("title_contains"),
            role: opt("role"),
            label_contains: label_contains.to_string(),
        },
        value: value.to_string(),
    };
    match crate::body::reflex::save(data_dir, &reflex).await {
        Ok(id) => tool_ok(&format!("learned reflex '{name}' ({id})")),
        Err(err) => tool_error(&err.to_string()),
    }
}

/// `name_person`: rename a person's cluster (face or voice) from its `id` (or
/// current key) to a learned `name` — the structural side of "we now know who
/// this is". Merges if the name already exists. See [`people_vectors::rename`].
async fn reflection_name_person(data_dir: &std::path::Path, args: &Value) -> Value {
    let id = args.get("id").and_then(Value::as_str).unwrap_or_default();
    let name = args.get("name").and_then(Value::as_str).unwrap_or_default();
    if id.trim().is_empty() || name.trim().is_empty() {
        return tool_error("name_person requires `id` and `name`");
    }
    match people_vectors::rename(data_dir, id, name).await {
        Ok(()) => tool_ok(&format!("named {id} → people/{name}")),
        Err(err) => tool_error(&err.to_string()),
    }
}

/// `merge_people`: fold the `from` cluster into `into` (same person, two keys —
/// across senses too, e.g. a voice id into an already-named face). See
/// [`people_vectors::rename`].
async fn reflection_merge_people(data_dir: &std::path::Path, args: &Value) -> Value {
    let from = args.get("from").and_then(Value::as_str).unwrap_or_default();
    let into = args.get("into").and_then(Value::as_str).unwrap_or_default();
    if from.trim().is_empty() || into.trim().is_empty() {
        return tool_error("merge_people requires `from` and `into`");
    }
    match people_vectors::rename(data_dir, from, into).await {
        Ok(()) => tool_ok(&format!("merged people/{from} → people/{into}")),
        Err(err) => tool_error(&err.to_string()),
    }
}

/// `keep_and_fade`: let a cold consolidated day's media fade to text, keeping the
/// spans the mind chose (see [`crate::mind::memory::decay::keep_and_fade`]). The safety
/// gate lives in the tool, so an attempt on an un-consolidated day comes back as a
/// tool error the session can read, not a panic.
async fn reflection_keep_and_fade(data_dir: &std::path::Path, args: &Value) -> Value {
    let Some(channel) = args.get("channel").and_then(Value::as_str) else {
        return tool_error("keep_and_fade requires `channel` (audio|vision)");
    };
    let Ok(channel) = channel.parse::<crate::types::Channel>() else {
        return tool_error(&format!("keep_and_fade: unknown channel {channel:?}"));
    };
    let date = args.get("date").and_then(Value::as_str).unwrap_or_default();
    if date.trim().is_empty() {
        return tool_error("keep_and_fade requires `date` (YYYY-MM-DD)");
    }
    let mut spans = Vec::new();
    if let Some(arr) = args.get("keep").and_then(Value::as_array) {
        for (i, item) in arr.iter().enumerate() {
            let parse = |k: &str| {
                item.get(k)
                    .and_then(Value::as_str)
                    .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                    .map(|t| t.with_timezone(&chrono::Utc))
            };
            let (Some(start), Some(end)) = (parse("start"), parse("end")) else {
                return tool_error(&format!(
                    "keep_and_fade: keep[{i}] needs RFC3339 `start` and `end`"
                ));
            };
            spans.push(crate::mind::memory::decay::KeepSpan { start, end });
        }
    }
    match crate::mind::memory::decay::keep_and_fade(data_dir, channel, date, &spans).await {
        Ok(r) => tool_ok(&format!(
            "faded {} {date}: kept {} keepsake(s), freed {} bytes",
            channel.as_str(),
            r.kept,
            r.bytes_freed
        )),
        Err(err) => tool_error(&err.to_string()),
    }
}

/// `image-text-to-text`: understand a stored still. Resolves the `ref` (the `⟨ref: …⟩` from a
/// `📷 photo arrived` signal, or one surfaced to reflection) to its bytes, then hands
/// it to [`perceive_still`] — which the bundle routes either to the model's own eyes
/// (native vision) or through the vision capability (text-only model).
async fn do_image_text_to_text(data_dir: &Path, args: &Value) -> Value {
    let prompt = args.get("prompt").and_then(Value::as_str).unwrap_or_default();
    let Some(reff) = args.get("ref").and_then(Value::as_str).filter(|s| !s.trim().is_empty()) else {
        return tool_error(
            "image-text-to-text needs `ref` — the ⟨ref: …⟩ from the image's signal, e.g. 2026-06-25/14/23-07.jpg",
        );
    };
    let Some((ts, rel, mime)) = parse_still_ref(reff) else {
        return tool_error(&format!(
            "image-text-to-text: malformed ref {reff:?} (expected <YYYY-MM-DD>/<HH>/<MM>-<SS>.<ext>)"
        ));
    };
    let Some(path) =
        crate::mind::memory::media::resolve(data_dir, crate::types::Channel::Vision, ts, &rel).await
    else {
        return tool_error(&format!("image-text-to-text: no media at {reff} (it may have faded)"));
    };
    let bytes = match tokio::fs::read(&path).await {
        Ok(b) => Bytes::from(b),
        Err(e) => return tool_error(&format!("image-text-to-text: reading {reff} failed: {e}")),
    };
    perceive_still(data_dir, bytes, &mime, prompt).await
}

/// `video-text-to-text`: understand a short span of the live camera. Reads the
/// in-progress (not-yet-flushed) minute from [`PartialMinute`] — the freshest source —
/// optionally trims it to the requested tail with ffmpeg, and hands the clip to
/// [`perceive_clip`]. Errors plainly when no camera is streaming, so the model can
/// ask the person to turn it on.
async fn do_video_text_to_text(
    data_dir: &Path,
    video_partial: &Mutex<Option<PartialMinute>>,
    args: &Value,
) -> Value {
    let prompt = args.get("prompt").and_then(Value::as_str).unwrap_or_default();
    let span = args.get("span").and_then(Value::as_str).unwrap_or_default();

    let Some((bytes, mime)) = partial_clip(video_partial) else {
        return tool_error(
            "no live camera to watch — `video-text-to-text` reads the camera streaming right now; \
             ask the person to turn it on, then try again.",
        );
    };

    // Trim to the requested tail when asked and ffmpeg can; on any trouble fall back
    // to the whole stretch (≤ ~1 min) rather than failing the look.
    let clip = match parse_last_secs(span) {
        Some(secs) => trim_tail(&bytes, &mime, secs).await.unwrap_or(bytes),
        None => bytes,
    };
    perceive_clip(data_dir, clip, &mime, prompt).await
}

/// Understand a still per the current [`bundle`](crate::body::capabilities::bundle):
/// a native-vision model gets the raw image as a tool-result block to reason over; a
/// text-only model gets the vision capability's description as text.
async fn perceive_still(data_dir: &Path, bytes: Bytes, mime: &str, prompt: &str) -> Value {
    use crate::body::capabilities::bundle::{self, Handling, Modality};
    match bundle::current().handling(Modality::Image) {
        Handling::Native => {
            let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
            let hint = if prompt.trim().is_empty() {
                "the still you asked to see".to_string()
            } else {
                format!("the still you asked to see — you wanted to know: {prompt}")
            };
            json!({
                "content": [
                    { "type": "text", "text": hint },
                    { "type": "image", "data": b64, "mimeType": mime },
                ],
                "isError": false,
            })
        }
        Handling::Polyfill => {
            use crate::body::capabilities::vision as vision_cap;
            if !vision_cap::available() {
                return tool_error("can't see stills here — no vision provider configured (set a vision key in Settings)");
            }
            let q = if prompt.trim().is_empty() { "Describe what you see." } else { prompt };
            match vision_cap::image_text_to_text(bytes, mime, q).await {
                Ok(text) => tool_ok(&text),
                Err(e) => {
                    crate::foundation::energy_state::note_402_error(data_dir, &e);
                    tool_error(&format!("vision understanding failed: {e}"))
                }
            }
        }
    }
}

/// Understand a short video clip. Always polyfilled — no model reached through the
/// adapter takes video — so the clip goes to the vision capability and the answer
/// comes back as text.
async fn perceive_clip(data_dir: &Path, bytes: Bytes, mime: &str, prompt: &str) -> Value {
    use crate::body::capabilities::bundle::{self, Modality};
    use crate::body::capabilities::vision as vision_cap;
    // The bundle always polyfills video today — no adapter path carries video to the
    // model — so this is the only arm; consulting `handling` keeps the
    // native-vs-polyfill decision in one place for the day a native-video model lands.
    let _ = bundle::current().handling(Modality::Video);
    if !vision_cap::available() {
        return tool_error("can't watch video here — no vision provider configured (set a vision key in Settings)");
    }
    let q = if prompt.trim().is_empty() { "Describe what happens in this clip." } else { prompt };
    match vision_cap::video_text_to_text(bytes, mime, q).await {
        Ok(text) => tool_ok(&text),
        Err(e) => {
            crate::foundation::energy_state::note_402_error(data_dir, &e);
            tool_error(&format!("video understanding failed: {e}"))
        }
    }
}

/// Concatenate the conversation's in-progress minute (`init` + `buf`) into one
/// independently-decodable clip, with its container mime. `None` when no camera is
/// streaming for the conversation.
fn partial_clip(map: &Mutex<Option<PartialMinute>>) -> Option<(Bytes, String)> {
    let guard = map.lock().unwrap();
    let p = guard.as_ref()?;
    let mut v = Vec::with_capacity(p.init.len() + p.buf.len());
    v.extend_from_slice(&p.init);
    v.extend_from_slice(&p.buf);
    Some((Bytes::from(v), p.mime.clone()))
}

/// Trim the last `secs` seconds out of an in-memory clip via ffmpeg. Writes the
/// bytes to a temp input file (ffmpeg needs a seekable input for `-sseof`), clips,
/// and cleans up. `Err` (no ffmpeg, undecodable) lets the caller send the whole clip.
async fn trim_tail(bytes: &Bytes, mime: &str, secs: f64) -> anyhow::Result<Bytes> {
    let ext = if mime.contains("mp4") { "mp4" } else { "webm" };
    let tmp = std::env::temp_dir().join(format!("hi-watch-{}.{ext}", uuid::Uuid::now_v7()));
    tokio::fs::write(&tmp, bytes).await?;
    let res = crate::foundation::vendors::ffmpeg_frame::clip_video(&tmp, -secs, secs).await;
    let _ = tokio::fs::remove_file(&tmp).await;
    res
}

/// Pull a tail length out of a `video-text-to-text` span like "last 20s" / "20 seconds" → 20.0.
/// `None` (no number) means "the whole recent stretch".
fn parse_last_secs(span: &str) -> Option<f64> {
    let digits: String = span
        .chars()
        .skip_while(|c| !c.is_ascii_digit())
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    digits.parse::<f64>().ok().filter(|n| *n > 0.0)
}

/// Map a still image extension to its MIME, for the native image content block.
fn ext_to_mime(ext: &str) -> String {
    match ext.to_ascii_lowercase().as_str() {
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "webp" => "image/webp",
        "gif" => "image/gif",
        _ => "application/octet-stream",
    }
    .to_string()
}

/// Parse a still `ref` — `<YYYY-MM-DD>/<HH>/<MM>-<SS>.<ext>` — into the timestamp and
/// channel-day-relative path [`crate::mind::memory::media::resolve`] wants, plus the
/// MIME. `None` if the shape doesn't match.
fn parse_still_ref(reff: &str) -> Option<(DateTime<Utc>, String, String)> {
    let (date, rel) = reff.split_once('/')?; // "2026-06-25", "14/23-07.jpg"
    let (hh, file) = rel.split_once('/')?; // "14", "23-07.jpg"
    let (stem, ext) = file.rsplit_once('.')?; // "23-07", "jpg"
    let (mm, ss) = stem.split_once('-')?; // "23", "07"
    // Reuse the proven RFC3339 parse (see keep_and_fade) rather than NaiveDate
    // helpers — a malformed part fails the parse and yields `None`.
    let ts = DateTime::parse_from_rfc3339(&format!("{date}T{hh}:{mm}:{ss}Z"))
        .ok()?
        .with_timezone(&Utc);
    Some((ts, rel.to_string(), ext_to_mime(ext)))
}

fn result(id: Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

fn error(id: Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

fn tool_ok(text: &str) -> Value {
    json!({ "content": [{ "type": "text", "text": text }], "isError": false })
}

fn tool_error(text: &str) -> Value {
    json!({ "content": [{ "type": "text", "text": text }], "isError": true })
}

/// A tool that is declared but not yet wired. `seam` names what is actually absent
/// and `provider` reports whether a vendor is configured *right now* — both checked,
/// neither guessed. The distinction is the whole point: a tool that reports a cause
/// it did not verify gets that cause written into memory as a fact, and the wrong
/// reason outlives the wrong result.
fn not_implemented(task: &str, seam: &str, provider: bool) -> Value {
    let provider =
        if provider { "a provider is configured" } else { "no provider is configured" };
    tool_error(&format!(
        "`{task}` is not implemented yet — {seam}. Right now {provider}. The tool's shape is \
         final and nothing about this call was wrong."
    ))
}

/// A view ref is a relative path under the views tree, naming the view's source file
/// minus the `.jsx` — e.g. `badminton-top10/leader` → `views/badminton-top10/
/// leader.jsx`. Each `/`-separated segment is a slug (letters, digits, `-`, `_`) —
/// no dots, no empty segments — so the ref stays inside the views tree and can't
/// traverse out. The build sub-agent writes `<ref>.jsx` with its own file tools (no
/// MCP tool needed); this reads it back server-side, so the JSX never enters the
/// mind's context.
fn valid_view_ref(view_ref: &str) -> bool {
    !view_ref.is_empty()
        && view_ref.len() <= 128
        && view_ref.split('/').all(|seg| {
            !seg.is_empty()
                && seg.bytes().all(|b| matches!(b, b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_'))
        })
}

/// Resolve a view ref to its stored JSX source (and what the builder declared
/// about it, if anything), read from the views tree. The agent passes only the
/// tiny ref through `show`; this reads the component back, plus an optional
/// `<ref>.geom.json` sidecar the builder wrote next to it. A missing or
/// unparseable sidecar is not an error — it just means host-owned captions, which
/// is the safe default. The sidecar no longer carries placement: every view is
/// full-bleed, so `owns_captions` is the only thing left to declare.
async fn resolve_view_ref(
    data_dir: &std::path::Path,
    view_ref: &str,
) -> Result<(String, Option<ViewTraits>), String> {
    let view_ref = view_ref.trim();
    if !valid_view_ref(view_ref) {
        return Err(format!("invalid ref `{view_ref}` (names and `/` only, no dots)"));
    }
    let views = data_dir.join("views");
    let source = tokio::fs::read_to_string(views.join(format!("{view_ref}.jsx")))
        .await
        .map_err(|e| format!("no such view ({e})"))?;
    let traits = match tokio::fs::read(views.join(format!("{view_ref}.geom.json"))).await {
        Ok(bytes) => serde_json::from_slice::<ViewTraits>(&bytes).ok(),
        Err(_) => None,
    };
    Ok((source, traits))
}

#[cfg(test)]
mod view_store_tests {
    use super::*;

    #[test]
    fn ref_validation_allows_nested_slugs_blocks_traversal() {
        assert!(valid_view_ref("badminton-top10"));
        assert!(valid_view_ref("badminton-top10/leader"));
        assert!(valid_view_ref("a/b/c_2"));
        assert!(!valid_view_ref(""), "empty");
        assert!(!valid_view_ref("../etc/passwd"), "dots blocked");
        assert!(!valid_view_ref("a//b"), "empty segment");
        assert!(!valid_view_ref("dot.name"), "dot blocked");
        assert!(!valid_view_ref("/abs"), "leading slash → empty segment");
        assert!(!valid_view_ref(&"x".repeat(129)), "too long");
    }

    #[tokio::test]
    async fn resolve_reads_views_source() {
        let dir = tempfile::tempdir().unwrap();
        let proj = dir.path().join("views").join("deck");
        tokio::fs::create_dir_all(&proj).await.unwrap();
        tokio::fs::write(proj.join("leader.jsx"), "export default () => 1").await.unwrap();
        let (source, traits) = resolve_view_ref(dir.path(), "deck/leader").await.unwrap();
        assert_eq!(source, "export default () => 1");
        // No sidecar written → host-owned captions.
        assert!(traits.is_none());
    }

    #[tokio::test]
    async fn resolve_reads_traits_sidecar() {
        let dir = tempfile::tempdir().unwrap();
        let proj = dir.path().join("views").join("deck");
        tokio::fs::create_dir_all(&proj).await.unwrap();
        tokio::fs::write(proj.join("leader.jsx"), "export default () => 1").await.unwrap();
        tokio::fs::write(proj.join("leader.geom.json"), r#"{"owns_captions":true}"#)
            .await
            .unwrap();
        let (_, traits) = resolve_view_ref(dir.path(), "deck/leader").await.unwrap();
        assert!(traits.expect("sidecar traits").owns_captions);
    }

    /// Sidecars written under the old placement schema are still on disk in every
    /// existing workshop. They must degrade to the default rather than failing the
    /// show: the unknown `region`/`size` keys are ignored and `owns_captions` — the
    /// one field that survived — is read straight through.
    #[tokio::test]
    async fn resolve_ignores_retired_placement_keys() {
        let dir = tempfile::tempdir().unwrap();
        let proj = dir.path().join("views").join("deck");
        tokio::fs::create_dir_all(&proj).await.unwrap();
        tokio::fs::write(proj.join("leader.jsx"), "export default () => 1").await.unwrap();
        tokio::fs::write(
            proj.join("leader.geom.json"),
            r#"{"region":"right","size":"wide","owns_captions":true}"#,
        )
        .await
        .unwrap();
        let (_, traits) = resolve_view_ref(dir.path(), "deck/leader").await.unwrap();
        assert!(traits.expect("sidecar traits").owns_captions);
    }

    #[tokio::test]
    async fn resolve_rejects_bad_refs() {
        let dir = tempfile::tempdir().unwrap();
        assert!(resolve_view_ref(dir.path(), "../secret").await.is_err(), "traversal");
        assert!(resolve_view_ref(dir.path(), "missing/view").await.is_err(), "no file");
    }
}

#[cfg(test)]
mod name_tests {
    use super::*;

    #[tokio::test]
    async fn name_person_renames_a_cluster_to_the_name() {
        let dir = tempfile::tempdir().unwrap();
        // A clustered-but-unnamed face id with a gallery.
        people_vectors::enroll(dir.path(), "ff32ce3w", people_vectors::Modality::Face, &[1.0, 0.0], b"m", "jpg")
            .await
            .unwrap();
        let r = reflection_name_person(
            dir.path(),
            &json!({ "id": "ff32ce3w", "name": "赵力" }),
        )
        .await;
        assert_eq!(r["isError"], false);
        let got = people_vectors::nearest(dir.path(), people_vectors::Modality::Face, &[1.0, 0.0], 1)
            .await
            .unwrap();
        assert_eq!(got[0].subject, "赵力");
    }

    #[tokio::test]
    async fn name_person_renames_a_voice_cluster_too() {
        let dir = tempfile::tempdir().unwrap();
        // A clustered-but-unnamed voice id with a gallery.
        people_vectors::enroll(dir.path(), "ab12cd34", people_vectors::Modality::Voice, &[1.0, 0.0], b"m", "wav")
            .await
            .unwrap();
        let r = reflection_name_person(
            dir.path(),
            &json!({ "id": "ab12cd34", "name": "赵力" }),
        )
        .await;
        assert_eq!(r["isError"], false);
        let got = people_vectors::nearest(dir.path(), people_vectors::Modality::Voice, &[1.0, 0.0], 1)
            .await
            .unwrap();
        assert_eq!(got[0].subject, "赵力");
    }

    #[tokio::test]
    async fn merge_people_ties_a_voice_id_to_a_named_face() {
        let dir = tempfile::tempdir().unwrap();
        // 赵力 is already known by face; their voice is still a separate opaque id.
        people_vectors::enroll(dir.path(), "赵力", people_vectors::Modality::Face, &[1.0, 0.0], b"m", "jpg")
            .await
            .unwrap();
        people_vectors::enroll(dir.path(), "ab12cd34", people_vectors::Modality::Voice, &[0.0, 1.0], b"m", "wav")
            .await
            .unwrap();
        let r = reflection_merge_people(
            dir.path(),
            &json!({ "from": "ab12cd34", "into": "赵力" }),
        )
        .await;
        assert_eq!(r["isError"], false);
        // 赵力 is now recognized by BOTH senses — the cross-modal bind.
        let face = people_vectors::nearest(dir.path(), people_vectors::Modality::Face, &[1.0, 0.0], 1)
            .await
            .unwrap();
        let voice = people_vectors::nearest(dir.path(), people_vectors::Modality::Voice, &[0.0, 1.0], 1)
            .await
            .unwrap();
        assert_eq!(face[0].subject, "赵力");
        assert_eq!(voice[0].subject, "赵力");
    }

    #[tokio::test]
    async fn name_person_rejects_blank_args() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(reflection_name_person(dir.path(), &json!({ "id": "x" })).await["isError"], true);
        assert_eq!(reflection_name_person(dir.path(), &json!({ "name": "y" })).await["isError"], true);
    }
}

#[cfg(test)]
mod screen_tool_tests {
    use super::*;
    use crate::body::capabilities::input::{Key, Modifier};

    #[test]
    fn png_dimensions_reads_ihdr() {
        let mut png = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
        png.extend_from_slice(&[0, 0, 0, 13]); // IHDR chunk length
        png.extend_from_slice(b"IHDR");
        png.extend_from_slice(&256u32.to_be_bytes());
        png.extend_from_slice(&128u32.to_be_bytes());
        assert_eq!(png_dimensions(&png), Some((256, 128)));
        assert_eq!(png_dimensions(b"not a png at all"), None);
        assert_eq!(png_dimensions(b"short"), None);
    }

    #[test]
    fn parse_key_handles_names_and_single_chars() {
        assert_eq!(parse_key("return"), Some(Key::Return));
        assert_eq!(parse_key("ENTER"), Some(Key::Return));
        assert_eq!(parse_key("esc"), Some(Key::Escape));
        assert_eq!(parse_key("a"), Some(Key::Char('a')));
        assert_eq!(parse_key("/"), Some(Key::Char('/')));
        assert_eq!(parse_key("f1"), None);
        assert_eq!(parse_key(""), None);
    }

    #[test]
    fn parse_mods_maps_aliases_and_drops_unknown() {
        let v = json!(["cmd", "Shift", "alt", "ctrl", "bogus"]);
        assert_eq!(
            parse_mods(Some(&v)),
            vec![Modifier::Command, Modifier::Shift, Modifier::Option, Modifier::Control]
        );
        assert_eq!(parse_mods(None), Vec::<Modifier>::new());
    }
}

#[cfg(test)]
mod vision_tool_tests {
    use super::*;

    #[test]
    fn parses_a_well_formed_still_ref() {
        let (ts, rel, mime) = parse_still_ref("2026-06-25/14/23-07.jpg").unwrap();
        assert_eq!(rel, "14/23-07.jpg");
        assert_eq!(mime, "image/jpeg");
        assert_eq!(ts.to_rfc3339(), "2026-06-25T14:23:07+00:00");
    }

    #[test]
    fn rejects_malformed_still_refs() {
        assert!(parse_still_ref("not-a-ref").is_none());
        assert!(parse_still_ref("2026-06-25/14/23.jpg").is_none(), "minute file, not a one-off still");
        assert!(parse_still_ref("2026-06-25/14/23-07").is_none(), "no extension");
    }

    #[test]
    fn ext_to_mime_covers_common_stills() {
        assert_eq!(ext_to_mime("JPG"), "image/jpeg");
        assert_eq!(ext_to_mime("png"), "image/png");
        assert_eq!(ext_to_mime("bin"), "application/octet-stream");
    }

    #[test]
    fn parse_last_secs_pulls_a_tail_length() {
        assert_eq!(parse_last_secs("last 20s"), Some(20.0));
        assert_eq!(parse_last_secs("30 seconds"), Some(30.0));
        assert_eq!(parse_last_secs("what just happened"), None);
        assert_eq!(parse_last_secs(""), None);
    }
}

#[cfg(test)]
mod surface_tests {
    use super::*;

    fn names(role: Option<&str>) -> Vec<String> {
        tools_for_role(role)
            .iter()
            .filter_map(|t| t.get("name")?.as_str().map(str::to_string))
            .collect()
    }

    /// Reaction's whole surface, pinned. `say` lived in the unreachable fallback arm
    /// for the entire life of the reaction/cognition split — defined, dispatchable, and
    /// advertised to nobody — so the voice fell back to plain message text. Nothing
    /// failed; it just quietly stopped being a call that returns.
    #[test]
    fn reaction_holds_say_and_show_and_nothing_else() {
        let mut got = names(Some("reaction"));
        got.sort();
        assert_eq!(
            got,
            vec!["say".to_string(), "send_message".to_string(), "show".to_string()],
            "its two expression channels, plus the one verb that reaches another agent"
        );
    }

    /// The other half of "and nothing else": a worker must not be able to speak.
    /// The one verb has to be on every rung, or an agent is unreachable by design.
    #[test]
    fn every_role_can_send_a_message() {
        for role in [
            Some("reaction"),
            Some("worker"),
            Some("deliberation"),
            Some("cognition"),
            Some("reflection"),
        ] {
            assert!(
                names(role).contains(&"send_message".to_string()),
                "{role:?} must hold send_message"
            );
        }
    }

    /// Deliberation's whole surface, pinned — and the reason it needed pinning: the
    /// rung was **opened as `SessionRole::Worker`**, so its `X-HI-Role` said `worker`
    /// and it was handed the effector kit (`look`, `act`, `video-text-to-text`) that belongs to the
    /// rung that does the job. It reads and it hands up; that is one tool.
    ///
    /// No `session_status`: status reads belong to owners, and Deliberation owns
    /// nothing. The hand-up to Cognition is one-way on purpose (invariant 3), so the
    /// answer arrives as mail rather than as something to poll for.
    #[test]
    fn deliberation_hands_up_and_holds_nothing_else() {
        assert_eq!(names(Some("deliberation")), vec!["send_message".to_string()]);
    }

    /// An unknown role gets **nothing**, and that is the point.
    ///
    /// This arm used to hold the legacy agentic reaction's kit — `say`, `show`,
    /// `record_reflex`, and the understanding tools — with a comment saying no live role
    /// mapped here. It was not merely dead: `SessionRole::Deliberation` stringifies to
    /// `deliberation`, which had no arm, so the moment that role was constructed it
    /// would have landed here and been handed `say` (refused at dispatch) and no
    /// `send_message` at all. A fallback that hands out someone else's surface turns a
    /// missing arm into a silently wrong one.
    #[test]
    fn an_unknown_role_gets_no_tools() {
        assert!(names(None).is_empty());
        assert!(names(Some("nonesuch")).is_empty());
    }

    /// Every role hi-agent actually opens has its own arm. The guard is the enum: if a
    /// variant is added and its arm is not, this fails rather than the session silently
    /// degrading to the empty fallback above.
    #[test]
    fn every_session_role_has_its_own_arm() {
        for role in ["reaction", "worker", "deliberation", "cognition", "reflection"] {
            assert!(!names(Some(role)).is_empty(), "`{role}` fell through to the empty fallback");
        }
    }

    /// One dispatcher. A the voice that could create workers would be a second one,
    /// spawning against Cognition unseen.
    /// Cognition's whole surface, pinned. Before it had an arm it fell into the `_`
    /// legacy fallback, which handed it `say` and `show` — refused at dispatch — and
    /// **not** `create_worker`, the one tool it exists to use. A rung with no arm is not
    /// a rung with defaults; it is a rung with someone else's.
    #[test]
    fn cognition_holds_the_switchboard_and_nothing_else() {
        let mut got = names(Some("cognition"));
        got.sort();
        assert_eq!(
            got,
            vec![
                "create_worker".to_string(),
                "send_message".to_string(),
                "session_messages".to_string(),
                "session_status".to_string(),
            ],
            "it delegates rather than does, and it has no mouth"
        );
    }

    #[test]
    fn only_the_standing_rungs_create_workers() {
        assert!(names(Some("reflection")).contains(&"create_worker".to_string()));
        assert!(names(Some("cognition")).contains(&"create_worker".to_string()));
        for role in [Some("reaction"), Some("worker")] {
            assert!(
                !names(role).contains(&"create_worker".to_string()),
                "{role:?} must not create workers"
            );
        }
    }

    #[test]
    fn no_other_role_can_speak() {
        for role in [Some("worker"), Some("reflection")] {
            assert!(!names(role).contains(&"say".to_string()), "{role:?} must not hold say");
        }
    }

    /// The modality surface is exactly six Hugging Face task names. Pinned as a set
    /// because the naming *is* the contract here: a seventh spelling of "look at this
    /// picture" is how one capability ends up with two names and a tool ends up
    /// guessing which of them it was handed.
    #[test]
    fn the_modality_surface_is_six_hugging_face_tasks() {
        let mut got: Vec<String> = [
            image_text_to_text_tool(),
            video_text_to_text_tool(),
            text_to_image_tool(),
            image_to_image_tool(),
            text_to_video_tool(),
            image_to_video_tool(),
        ]
        .iter()
        .map(|t| t["name"].as_str().unwrap().to_string())
        .collect();
        got.sort();
        assert_eq!(
            got,
            vec![
                "image-text-to-text",
                "image-to-image",
                "image-to-video",
                "text-to-image",
                "text-to-video",
                "video-text-to-text",
            ]
        );
    }

    /// Understanding is advertised where it was before the rename; generation goes to
    /// the rung that produces artifacts. Pinned so a stub landing on Reaction — the one
    /// surface that is a hard rail — fails here rather than in a conversation.
    #[test]
    fn generation_belongs_to_workers_and_reaction_stays_a_voice() {
        let worker = names(Some("worker"));
        for task in ["text-to-image", "image-to-image", "text-to-video", "image-to-video"] {
            assert!(worker.contains(&task.to_string()), "worker must hold `{task}`");
            for role in [Some("reaction"), Some("deliberation"), Some("cognition")] {
                assert!(!names(role).contains(&task.to_string()), "{role:?} must not hold `{task}`");
            }
        }
        assert!(worker.contains(&"video-text-to-text".to_string()));
        assert!(names(Some("reflection")).contains(&"image-text-to-text".to_string()));
    }

    /// Every advertised tool must dispatch. The four generation tasks are declared
    /// before they are wired, so the failure this guards is real: a tool in the surface
    /// with no arm falls through to "unknown tool", which tells the model its call was
    /// malformed when the truth is the feature does not exist yet. Reporting a cause
    /// you did not verify is how a wrong reason outlives a wrong result.
    #[tokio::test]
    async fn the_generation_tasks_dispatch_to_a_not_implemented_error() {
        let dir = tempfile::tempdir().unwrap();
        let tools = crate::body::reaction::ToolRegistry::new();
        let partial = Mutex::new(None);
        let obs = Observatory::new(None);

        for name in ["text-to-image", "image-to-image", "text-to-video", "image-to-video"] {
            let got = dispatch_tool(
                &tools,
                dir.path(),
                &partial,
                &obs,
                Some(7),
                Some("worker"),
                name,
                &json!({ "prompt": "a cat", "ref": "2026-06-25/14/23-07.jpg" }),
            )
            .await;
            assert_eq!(got.get("isError").and_then(Value::as_bool), Some(true), "{name}");
            let text = got["content"][0]["text"].as_str().unwrap();
            assert!(text.contains("not implemented yet"), "{name} got: {text}");
            assert!(text.contains(name), "{name} must name itself: {text}");
            assert!(!text.contains("unknown tool"), "{name} fell through to the fallback: {text}");
        }
    }

    /// Surface membership is a context optimization, not a rail — so the rungs that
    /// must not dispatch are refused at *dispatch*, whatever their model emits.
    ///
    /// This was enforced only by accident until Reaction was given a session id: the
    /// identity check rejected it for having no `X-HI-Session-Id`, which reads as a
    /// guard and is not one. A rung with an identity and no advertised `create_worker`
    /// could call it anyway.
    #[tokio::test]
    async fn create_worker_is_refused_to_non_owner_roles_at_dispatch() {
        let dir = tempfile::tempdir().unwrap();
        let tools = crate::body::reaction::ToolRegistry::new();
        let partial = Mutex::new(None);
        let obs = Observatory::new(None);

        for role in [Some("reaction"), Some("worker"), Some("deliberation"), None] {
            let got = dispatch_tool(
                &tools,
                dir.path(),
                &partial,
                &obs,
                // An identity, so this cannot pass for the old accidental rejection.
                Some(7),
                role,
                "create_worker",
                &json!({ "task": "do a thing" }),
            )
            .await;
            assert_eq!(got.get("isError").and_then(Value::as_bool), Some(true), "{role:?}");
            let text = got["content"][0]["text"].as_str().unwrap();
            assert!(text.contains("may not dispatch work"), "{role:?} got: {text}");
        }
    }

    /// The one verb crossing has to be *observable*, including when it fails. The send
    /// happens here in MCP, which held no observatory handle — so every agent-to-agent
    /// edge was invisible while workers were not, and the inspector showed the nodes of
    /// the graph and none of its arrows.
    ///
    /// A miss is the interesting case, so that is what this pins: nothing is live at
    /// `99`, and the event still lands carrying `delivery: unknown`.
    #[tokio::test]
    async fn a_send_that_reaches_nobody_is_still_recorded_as_an_edge() {
        let dir = tempfile::tempdir().unwrap();
        let tools = crate::body::reaction::ToolRegistry::new();
        let partial = Mutex::new(None);
        let obs = Observatory::new(None);

        let got = dispatch_tool(
            &tools,
            dir.path(),
            &partial,
            &obs,
                Some(7),
            Some("reaction"),
            "send_message",
            &json!({ "to": "99", "message": "are you there" }),
        )
        .await;
        assert_eq!(got.get("isError").and_then(Value::as_bool), Some(true));

        let (replay, _rx) = obs.subscribe().await;
        assert_eq!(replay.len(), 1, "the failed edge is history too");
        let v = serde_json::to_value(&replay[0]).unwrap();
        assert_eq!(v["event"], "message_sent");
        assert_eq!(v["from"], 7);
        assert_eq!(v["to"], 99);
        assert_eq!(v["delivery"], "unknown");
        assert_eq!(v["message"], "are you there");
    }
}
