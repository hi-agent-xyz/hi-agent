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

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use bytes::Bytes;

use crate::body::capabilities::{image_gen, video_gen, view_render};
use crate::body::reaction::{LoopControl, ToolOwner, ToolRegistry};
use crate::foundation::observatory::{EventKind, Observatory};
use crate::foundation::registry;
use crate::identity::{Role, WorkerType};
use crate::mind::memory::people_vectors;
use crate::foundation::server::PartialMinute;

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
/// `hi_show` (it speaks via plain message text, not a tool); a worker gets the
/// work tools but no voice; reflection reads/writes derived memory. The `_` fallback
/// is the legacy agentic reaction's full toolset, kept for untagged sessions.
/// The `hi_say` tool — Reaction's voice.
///
/// Speech is a **call, not message text**, and that is the whole point: a call returns.
/// The host can hold an utterance until the room is right, queue it behind another, or
/// refuse it — and Reaction finds out which. Text streamed into the transcript is
/// fire-and-forget and leaves nowhere for that decision to live.
///
/// It is also where a **promise** is made, via `back_in`, and that is not a second
/// concern smuggled onto one tool: a promise is only a promise once it has been said,
/// so the size of a silence belongs to the utterance that opened it. The alternative —
/// a separate verb — could arm a wake for a number nobody was ever told.
fn say_tool() -> Value {
    tool(
        "hi_say",
        "Speak to the person. Everything you want said aloud goes through this tool — \
         plain text you write is NOT spoken. Call it with one natural chunk at a time, \
         keeping each call under about 240 characters; an overlong call returns too_long \
         and is not sent. Several accepted calls in a turn are spoken in order. To stay \
         silent, don't call it at all. It tells you where the words actually landed — \
         aloud, on screen only, or waiting for them to come back — so you can judge \
         whether a spoken line was worth spending.",
        json!({
            "type": "object",
            "properties": {
                "text": { "type": "string", "description": "What to say, as natural spoken language (no markdown)." },
                "back_in": {
                    "type": "string",
                    "description": "Optional, and the only timer you have. When you put a size \
                                    on a silence — \"give me ten minutes\" — put that same size \
                                    here (`90s`, `10m`, `1h`) and you will be woken when it is \
                                    up, unless something has already brought you back by then. \
                                    Without it the number you named is only words, and they \
                                    find out where things stand by asking. Set it on the \
                                    utterance that makes the promise, and set it again each \
                                    time you give them a new number.",
                },
            },
            "required": ["text"],
        }),
    )
}

/// `SendMessage` — the one verb between agents.
///
/// One direction, no reply. A reply is this same call going the other way, which is why
/// the sender is stamped host-side from the calling session rather than passed in.
///
/// **The description names the three rungs outright.** It used to say an address is "a
/// number" and that nobody is reachable by name, which was true of ordinals and made the
/// roster the only way to learn an address — so a rung whose prompt described the act
/// without naming this tool had nothing to go on but the tool list, and the agent runtime's
/// own `send_message` sits in that list too.
fn send_message_tool() -> Value {
    tool(
        "hi_send_message",
        "Send a message to another agent session. One direction — it does not wait for a \
         reply, and the return value only tells you whether it was delivered. If you want an \
         answer, the other side sends you one the same way; your identity travels with the \
         message so it knows where to reach you. `to` is always a **session id**. The three \
         standing rungs are `reaction` (the voice), `cognition` (the brain) and `reflection` \
         — one of each, always those names. A worker's id comes back from \
         `hi_create_worker` and looks like `view-builder-kyoto-trip`; a message you received \
         carries its sender's. Everyone you may reach right now is listed in your window \
         under \"Who you can reach right now\", each with its id.",
        json!({
            "type": "object",
            "properties": {
                "to": {
                    "type": "string",
                    "description": "A session id: `reaction`, `cognition`, `reflection`, or a worker's.",
                },
                "message": { "type": "string", "description": "What you want them to know, in plain words." },
            },
            "required": ["to", "message"],
        }),
    )
}

fn create_worker_tool() -> Value {
    tool(
        "hi_create_worker",
        "Start a working session to carry out a job, and get back its session id. It runs \
         with the full toolset and no voice of its own; it reports to you and to nobody else. \
         Send it the brief with `hi_send_message`, ask how it is doing with `hi_session_status`, and \
         read what it has produced with `hi_session_messages`. **It is yours until you end it**: \
         a session that has reported is not finished, it is waiting, and it keeps its whole \
         context for the next thing you send it however long that takes. Nothing reclaims it \
         on a timer, so when its errand is genuinely done, `hi_close_worker` it — every session \
         you leave open holds a subprocess.",
        json!({
            "type": "object",
            "properties": {
                "title": {
                    "type": "string",
                    "description": "What this errand is, in **one short line** — how you would \
                                    name it to a colleague in passing, not the first sentence of \
                                    the brief. This is the only part of this call anyone ever \
                                    reads: it is the line the session shows up as on the \
                                    person's screen, in your own window, and in the offer made \
                                    back to you if a restart kills it. So write the subject and \
                                    the verb — \"recover the stalled xyz deploy\", \"chase the \
                                    group listener that went quiet\" — and leave out paths, ids, \
                                    digests and preamble. 40-60 characters is the target; past \
                                    72 it is cut.",
                },
                "task": {
                    "type": "string",
                    "description": "A self-contained description of the work, at whatever length \
                                    it needs. It becomes the session's first prompt and is read \
                                    by the worker alone, so nothing in it has to be short — \
                                    `title` is the short version.",
                },
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
                                    into the drive, `person-reader` to read one person out of \
                                    the record and write their facet — the settling pass's, and \
                                    not yours to start unless you are it.",
                },
                "subject": {
                    "type": "string",
                    "description": "The ledger subject of the task this errand is for — the \
                                    directory name under `memory/facets/tasks/`, not the title. \
                                    Set it whenever the work belongs to a task you are tracking: \
                                    it is what makes that task show as *being worked on* rather \
                                    than as owed by nobody, and what lets a later glance tell a \
                                    task with someone on it from one that has quietly stalled. \
                                    Leave it out only for work that is genuinely not in the \
                                    ledger.",
                },
                "resume": {
                    "type": "string",
                    "description": "Pick an errand back up where a restart cut it off, instead \
                                    of starting cold. Only a thread from the offer in your first \
                                    pulse after the host starts — anything else is refused. The \
                                    session opens remembering what that one was doing, so `task` \
                                    should say what has changed and what is left, not restate \
                                    the job. Leave it out for new work, and for an errand stale \
                                    enough that its half-done state is a liability.",
                },
            },
            "required": ["title", "task"],
        }),
    )
}

/// `hi_cancel_worker` — the other half of `hi_create_worker`.
///
/// Without it, work handed out could not be taken back: everything else that reaches a
/// working session is a message, and a message is read between turns, so a "stop" sent
/// that way arrives after the thing it was meant to stop. This reaches into the running
/// turn and ends it.
///
/// It is deliberately *not* destructive. The session survives the cancel with its whole
/// context, so the common shape — "no, not that, do this instead" — is a cancel followed
/// by `hi_send_message` to the same id, and the session already knows everything it learned
/// before being stopped.
fn cancel_worker_tool() -> Value {
    tool(
        "hi_cancel_worker",
        "Stop a working session you created, now, mid-work. Use it the moment the person \
         takes something back or changes direction — telling them you have stopped without \
         calling this is a sentence, not a stop, and the work carries on. The session stays \
         alive and keeps everything it has learned, so to redirect rather than drop the \
         work, cancel and then `hi_send_message` the new instruction to the same id.",
        json!({
            "type": "object",
            "properties": { "id": { "type": "string", "description": "The session id." } },
            "required": ["id"],
        }),
    )
}

/// `hi_close_worker` — the verb that used to be a timer.
///
/// A working session held a subprocess until fifteen idle minutes killed it, which meant
/// "I am finished with this session" was said by a clock that could not tell a finished
/// errand from a waiting one. It is a sentence the owner says now.
fn close_worker_tool() -> Value {
    tool(
        "hi_close_worker",
        "Finish with a working session for good, freeing what it holds. Nothing else ends \
         one — a session you leave open stays open, holding its context and its share of \
         the machine, until you say this. So close a session once its errand is genuinely \
         done and you will not be asking it anything more. Not the same as `hi_cancel_worker`: \
         that stops the current turn and *keeps* the session, which is what you want for \
         \"no, do this instead\". This ends it, and everything it learned goes with it — if \
         you may still want a follow-up, leave it open. A turn already running is allowed to \
         finish and report before the session closes.",
        json!({
            "type": "object",
            "properties": { "id": { "type": "string", "description": "The session id." } },
            "required": ["id"],
        }),
    )
}

fn session_status_tool() -> Value {
    tool(
        "hi_session_status",
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
        "hi_session_messages",
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

/// `hi_review_view` — render a saved view in a real browser and hand back both the
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
        "hi_review_view",
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
                "hi_look",
                "See the user's screen right now — returns a screenshot of the main display, plus \
                 its pixel size and the frontmost app. Use it to find where things are before you \
                 `hi_act`, and again after acting to confirm what changed. The positions you pass to \
                 `hi_act` are fractions of THIS image.",
                json!({ "type": "object", "properties": {} }),
            ),
            tool(
                "hi_act",
                "Operate the user's screen like a human would: move, click, type, or press keys. \
                 Positions are normalized fractions of the screen read off the latest `hi_look` — `x` \
                 is 0.0 (left) to 1.0 (right), `y` is 0.0 (top) to 1.0 (bottom). After you act, call \
                 `hi_look` again to check it worked.",
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
        // voice (its surface is the one hard rail) and Cognition reads and dispatches —
        // a worker is the only rung that produces artifacts.
        .chain(generation_tools())
        .collect(),
        // The reflection ("sleep") surface: a voice-less session that consolidates
        // the one raw frontier into derived memory.
        Some("reflection") => vec![
            send_message_tool(),
            create_worker_tool(),
            cancel_worker_tool(),
            close_worker_tool(),
            session_status_tool(),
            session_messages_tool(),
            tool(
                "hi_record_episode",
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
                "hi_read_facet",
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
                "hi_update_facet",
                "Write your whole current understanding of one subject — regenerate the file, don't patch \
                 it: pass the complete text (old understanding folded together with the new), not just a \
                 delta. Every claim should cite the episode(s) it came from by their refs (the values \
                 hi_record_episode returned). Dimensions are open-ended; reuse an existing dimension/subject \
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
                "hi_name_person",
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
                "hi_merge_people",
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
                "hi_keep_and_fade",
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
                "hi_update_proactivity",
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
        // The shared brain. It delegates rather than does, so its surface is the
        // switchboard and nothing else: hand work out, ask after it, read what came back.
        //
        // The ledger it owns needs no tool — a task is a plain facet on disk, and it has
        // the adapter's own Read/Write. That is why "sole writer of the ledger" is a
        // matter of which rung is *told* to write it, and why that instruction moving out
        // in `cognition.md` is what makes it true.
        //
        // **It reads with the adapter's own Read/Write, and that is why looking needs no
        // tool here.** A photo arrives as a ref, a ref is a path, and the rung that must
        // open it already can (`docs/arch/foundation.md`). The retired Deliberation had
        // exactly this surface minus the dispatch verbs; folding it in added nothing to
        // declare.
        //
        // No `hi_say`, no `hi_show`: it proposes, Reaction voices. Enforced three ways
        // that agree — absent here, refused at dispatch above, and its sink carries no
        // sequencer to express through.
        Some("cognition") => vec![
            send_message_tool(),
            create_worker_tool(),
            cancel_worker_tool(),
            close_worker_tool(),
            session_status_tool(),
            session_messages_tool(),
        ],
        // **Reaction** — the mouth. Its two expression channels plus the one verb that
        // reaches another agent, and nothing else: no reads, no fetches, no built-ins
        // (`docs/arch/agents.md#reaction`). A stale comment stood above the arm before
        // this one saying the voice "speaks via plain message text (not a `hi_say` tool)
        // and gets exactly one expression tool — `hi_show`", which had not been true since
        // `hi_say` was added here; it also sat directly above the *cognition* arm, so it
        // described the wrong rung in the wrong place.
        Some("reaction") => vec![say_tool(), show_tool(), send_message_tool()],
        // **Nothing.** Every role hi-agent opens is named above, so reaching here means
        // an unheadered or unknown session, and handing one an arbitrary toolset is how
        // the previous occupant of this arm survived: it held the legacy agentic
        // reaction's kit — `hi_say`, `hi_show`, `hi_record_reflex`, and the two understanding tools —
        // long after no live role mapped to it, and read as a live surface in every
        // review.
        //
        // One tool lost its only declaration with this and was already unreachable:
        // `hi_record_reflex`, which still has **no live role** — the recognizer and the
        // invoke route are real, so the reflex store can be read and fired but never
        // written. That is an open decision, not an oversight: it needs a rung or it
        // needs deleting, and it is now visibly nobody's rather than sitting in an arm
        // that looked live. (`alarm` was the other, and it is gone outright — the host
        // fires at no named time; see `docs/arch/host.md#glancing-up`.)
        _ => vec![],
    }
}

fn tool(name: &str, description: &str, input_schema: Value) -> Value {
    json!({ "name": name, "description": description, "inputSchema": input_schema })
}

// ---------------------------------------------------------------------------
// Modality tools
//
// Named for their Hugging Face task rather than for a verb — `hi_image_text_to_text`
// where this was once `see`. The task name states the *signature* (what goes in,
// what comes out) where a verb states only an intent, and it is the same string the
// provider model cards use, so a tool, its capability module and its config key all
// carry one name. The trade: the name no longer reads as an instruction, so each
// `description` has to open with the action.
//
// Six tasks, one axis that matters:
//
//   understanding  `hi_image_text_to_text`, `hi_video_text_to_text`  → goes through
//       [`bundle::Bundle`]: raw pixels when the model takes them natively, the
//       vision capability's text when it doesn't.
//   generation     `hi_text_to_image`, `hi_image_to_image`,
//                  `hi_text_to_video`, `hi_image_to_video`           → never touches the
//       bundle. No model reached through the agent wire emits pixels, so these are
//       always a provider call.
//
// The four generation tools are wired. The decision they waited on — where a
// generated artifact lands and what ref it gets — is settled: the bytes go to
// [`drive/`](crate::mind::memory::media::store_artifact), the tree that does not
// fade, and the ref is `drive/<path>`, a second arm on the one ref grammar rather
// than a second grammar. So `hi_image_to_image` takes a camera still, a handed file and
// its own last output through the same argument.
//
// They also differ from every other tool here in being **built per call**: the
// `model` argument is described from the live menu, because "the agent chooses the
// model" is only true if the agent is shown what there is.
// ---------------------------------------------------------------------------

/// `hi_image_text_to_text` — an image plus an instruction in, text out.
fn image_text_to_text_tool() -> Value {
    tool(
        "hi_image_text_to_text",
        "Look at a still image and answer about it — a photo the person sent, a screenshot they \
         handed over, or a frame held up to the camera. It reaches you as a signal carrying an \
         `⟨ref: …⟩`; pass that `ref`, and optionally what you want to know. Reach for it the moment \
         seeing the picture beats guessing: read a label/menu/handwriting, identify a thing, check \
         what's on a screen they photographed.",
        json!({
            "type": "object",
            "properties": {
                "ref": { "type": "string", "description": "The ⟨ref: …⟩ carried by the image's signal, e.g. vision/2026-06-25/14/23-07.jpg." },
                "prompt": { "type": "string", "description": "Optional: what you want to know about the image (a question or focus). Omit to just look." },
            },
            "required": ["ref"],
        }),
    )
}

/// `hi_video_text_to_text` — a span of live camera plus an instruction in, text out.
/// Always polyfilled: no model behind the adapter takes video, so the clip is
/// understood by the vision capability and the text handed back.
fn video_text_to_text_tool() -> Value {
    tool(
        "hi_video_text_to_text",
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

/// One menu entry, normalised out of whichever capability's `ModelInfo` it came from
/// — the two are separate types on purpose (independent capabilities) and this is the
/// only place that needs to treat them alike.
struct MenuEntry {
    name: String,
    quality: i64,
    speed: i64,
    price: i64,
}

/// The `model` property, described from what is **actually reachable right now**.
///
/// Choosing the model is the agent's job, which only works if it is shown the menu —
/// a name it was never told about is not a choice. The list comes from the credential
/// store's published models, so it tracks what the broker minted for this account
/// rather than a constant that rots.
///
/// The hints are ordinal, not raw scores. "highest quality" is a fact an agent can
/// act on; `quality: 87` is a number it has no scale for.
fn model_property(menu: Vec<MenuEntry>, default: Option<String>, verb: &str) -> Value {
    let mut description = format!("Optional: which model to {verb} with.");
    if menu.is_empty() {
        description.push_str(
            " No menu is published for this account, so any model name the provider \
             serves is passed through as given.",
        );
    } else {
        // Comparative tags only when there is something to compare against. One model
        // labelled "highest quality, fastest, cheapest" is three words of noise about
        // a choice that does not exist.
        let compare = menu.len() > 1;
        let tag = |f: fn(&MenuEntry) -> i64, want_max: bool| -> Option<String> {
            if !compare {
                return None;
            }
            let it = menu.iter().filter(|m| f(m) != 0);
            if want_max { it.max_by_key(|m| f(m)) } else { it.min_by_key(|m| f(m)) }
                .map(|m| m.name.clone())
        };
        let best = tag(|m| m.quality, true);
        let fastest = tag(|m| m.speed, true);
        let cheapest = tag(|m| m.price, false);
        let listed: Vec<String> = menu
            .iter()
            .map(|m| {
                let mut tags = Vec::new();
                if Some(&m.name) == best.as_ref() {
                    tags.push("highest quality");
                }
                if Some(&m.name) == fastest.as_ref() {
                    tags.push("fastest");
                }
                if Some(&m.name) == cheapest.as_ref() {
                    tags.push("cheapest");
                }
                if tags.is_empty() {
                    m.name.clone()
                } else {
                    format!("{} ({})", m.name, tags.join(", "))
                }
            })
            .collect();
        description.push_str(&format!(" Reachable now: {}.", listed.join("; ")));
    }
    match default {
        Some(d) => description.push_str(&format!(" Omit to use {d}.")),
        None => description.push_str(" Omit to use the provider's default."),
    }
    json!({ "type": "string", "description": description })
}

fn image_model_property(verb: &str) -> Value {
    let menu = image_gen::models()
        .into_iter()
        .map(|m| MenuEntry { name: m.name, quality: m.quality, speed: m.speed, price: m.price })
        .collect();
    model_property(menu, image_gen::default_model(), verb)
}

fn video_model_property() -> Value {
    let menu = video_gen::models()
        .into_iter()
        .map(|m| MenuEntry { name: m.name, quality: m.quality, speed: m.speed, price: m.price })
        .collect();
    model_property(menu, video_gen::default_model(), "generate")
}

/// `hi_text_to_image` — a prompt in, a new image out, filed in the drive.
fn text_to_image_tool() -> Value {
    tool(
        "hi_text_to_image",
        "Draw a new image from a description. Say what it should show; every other argument is \
         a knob you may set or leave alone, and leaving one alone means the model decides. The \
         picture is filed in the drive and you get back its `⟨ref: …⟩` — pass that to \
         `hi_image_to_image` to change it, to `hi_image_text_to_text` to look at it, or report it to \
         whoever asked so it can go on screen. A knob a model cannot honour comes back as an \
         error naming one that can, so nothing is silently ignored.",
        json!({
            "type": "object",
            "properties": {
                "prompt": { "type": "string", "description": "What the image should show." },
                "model": image_model_property("draw"),
                "size": { "type": "string", "description": "Optional: e.g. \"1024x1024\", \"2K\", \"adaptive\". gpt-image models want both edges to be multiples of 16." },
                "quality": { "type": "string", "description": "Optional cost/quality dial where the model has one: \"low\", \"medium\", \"high\"." },
                "n": { "type": "integer", "description": "Optional: how many images to return. Default one." },
                "background": { "type": "string", "description": "Optional: \"transparent\" for a cutout, \"opaque\", \"auto\". Not every model can." },
                "output_format": { "type": "string", "description": "Optional: \"png\", \"jpeg\", \"webp\"." },
                "seed": { "type": "integer", "description": "Optional: fix the seed to make the result repeatable." },
                "watermark": { "type": "boolean", "description": "Optional, doubao models only." },
            },
            "required": ["prompt"],
        }),
    )
}

/// `hi_image_to_image` — an existing image plus an instruction in, a new image out.
fn image_to_image_tool() -> Value {
    tool(
        "hi_image_to_image",
        "Edit an existing image — say what to change and it returns a new image, leaving the \
         original untouched. Pass the `⟨ref: …⟩` of the image to work from: a camera still, a \
         file someone handed over, or one you drew a moment ago. The result is filed in the \
         drive and comes back as its own ref, so you can edit that in turn.",
        json!({
            "type": "object",
            "properties": {
                "ref": { "type": "string", "description": "The ⟨ref: …⟩ of the image to edit, e.g. vision/2026-06-25/14/23-07.jpg or drive/generated/2026-06-25/142307-a-red-bicycle.png." },
                "prompt": { "type": "string", "description": "What to change (e.g. \"make the sky overcast\", \"remove the car\")." },
                "model": image_model_property("edit"),
                "size": { "type": "string", "description": "Optional output size." },
                "quality": { "type": "string", "description": "Optional cost/quality dial: \"low\", \"medium\", \"high\"." },
                "n": { "type": "integer", "description": "Optional: how many variants to return. Default one." },
                "background": { "type": "string", "description": "Optional: \"transparent\", \"opaque\", \"auto\"." },
                "output_format": { "type": "string", "description": "Optional: \"png\", \"jpeg\", \"webp\"." },
                "seed": { "type": "integer", "description": "Optional: fix the seed to make the result repeatable." },
            },
            "required": ["ref", "prompt"],
        }),
    )
}

/// `hi_text_to_video` — a prompt in, a clip out, mailed back when it lands.
fn text_to_video_tool() -> Value {
    tool(
        "hi_text_to_video",
        "Generate a short video clip from a description. Generation runs for minutes, so this \
         returns straight away and the finished clip arrives as a message carrying its \
         `⟨ref: …⟩`. There is nothing to poll: get on with something else and you will be told.",
        json!({
            "type": "object",
            "properties": {
                "prompt": { "type": "string", "description": "What the clip should show." },
                "model": video_model_property(),
                "duration": { "type": "integer", "description": "Optional: clip length in seconds." },
                "ratio": { "type": "string", "description": "Optional: aspect ratio, e.g. \"16:9\", \"9:16\", \"1:1\"." },
                "resolution": { "type": "string", "description": "Optional: e.g. \"480p\", \"720p\", \"1080p\"." },
                "seed": { "type": "integer", "description": "Optional: fix the seed to make the result repeatable." },
                "watermark": { "type": "boolean", "description": "Optional." },
            },
            "required": ["prompt"],
        }),
    )
}

/// `hi_image_to_video` — an image as first frame plus an optional prompt in, a clip out.
fn image_to_video_tool() -> Value {
    tool(
        "hi_image_to_video",
        "Animate an existing still — it becomes the first frame of a short clip. Pass the \
         `⟨ref: …⟩` of the image, and optionally say how it should move. Like any generation \
         this runs for minutes: it returns straight away and the clip arrives as a message \
         carrying its own ref.",
        json!({
            "type": "object",
            "properties": {
                "ref": { "type": "string", "description": "The ⟨ref: …⟩ of the still to animate from, e.g. vision/2026-06-25/14/23-07.jpg or one you generated." },
                "prompt": { "type": "string", "description": "Optional: how it should move or what should happen." },
                "model": video_model_property(),
                "duration": { "type": "integer", "description": "Optional: clip length in seconds." },
                "ratio": { "type": "string", "description": "Optional: aspect ratio, e.g. \"16:9\", \"9:16\", \"1:1\"." },
                "resolution": { "type": "string", "description": "Optional: e.g. \"480p\", \"720p\", \"1080p\"." },
                "seed": { "type": "integer", "description": "Optional: fix the seed to make the result repeatable." },
                "watermark": { "type": "boolean", "description": "Optional." },
            },
            "required": ["ref"],
        }),
    )
}

/// The four generation tasks, as one surface. Declared together because they share a
/// shape no other tool here has: they produce an artifact, file it in `drive/`, and
/// answer with a ref rather than with the thing itself.
///
/// **These four are built per call, not constant.** Their `model` argument is
/// described from the live menu ([`model_property`]), so a `tools/list` reflects what
/// this account can actually reach right now.
fn generation_tools() -> Vec<Value> {
    vec![text_to_image_tool(), image_to_image_tool(), text_to_video_tool(), image_to_video_tool()]
}

/// The `hi_show` tool — put a view on the screen. The reaction's one expression
/// tool beyond speech: it shows a view a worker already built (by `ref`), or a
/// trivial inline one. Shared by the reaction surface and the legacy fallback.
fn show_tool() -> Value {
    tool(
        "hi_show",
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
    session_id: Option<crate::foundation::registry::SessionId>,
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
    session_id: Option<crate::foundation::registry::SessionId>,
    role: Option<&str>,
    name: &str,
    args: &Value,
) -> Value {
    // Expression tools belong to the reaction alone — it is the single
    // guideline-carrying voice, so everything the person sees or hears goes through
    // its `reaction.md` generation. A worker or reflection session must never speak
    // or take the screen even if its model emits the call (these aren't in its
    // advertised surface); enforce that structurally here, not just via the tool list.
    if matches!(name, "hi_say" | "hi_show") && role != Some("reaction") {
        return tool_error(&format!(
            "`{name}` is reaction-only; role `{}` may not speak or show",
            role.unwrap_or("<none>")
        ));
    }

    // Reflection tools are pure derived-memory IO over `data_dir`; they don't touch
    // a loop sink, so handle them before the sink lookup.
    match name {
        "hi_record_episode" => return reflection_record_episode(data_dir, args).await,
        "hi_read_facet" => return reflection_read_facet(data_dir, args).await,
        "hi_update_facet" => return reflection_update_facet(data_dir, args).await,
        "hi_update_proactivity" => return reflection_update_proactivity(data_dir, args).await,
        "hi_name_person" => return reflection_name_person(data_dir, args).await,
        "hi_merge_people" => return reflection_merge_people(data_dir, args).await,
        "hi_keep_and_fade" => return reflection_keep_and_fade(data_dir, args).await,
        // Reachable by name only — `hi_record_reflex` is advertised to no role, because
        // the reflex rung is **deferred** (see `body::reflex`). Kept dispatchable rather
        // than deleted so the authoring half is one arm entry away when it gets a rung,
        // and so a session that somehow names it gets the real behaviour instead of
        // "unknown tool". Do not re-advertise it without taking that decision.
        "hi_record_reflex" => return reflex_record(data_dir, args).await,
        "hi_look" => return do_look().await,
        "hi_act" => return do_act(args).await,
        "hi_image_text_to_text" => return do_image_text_to_text(data_dir, args).await,
        "hi_video_text_to_text" => {
            return do_video_text_to_text(data_dir, video_partial, args).await;
        }
        // The four generation tasks. Each files what it makes into `drive/` and
        // answers with the ref; the two video ones answer immediately and mail the
        // clip to this session when it lands.
        "hi_text_to_image" => return do_text_to_image(data_dir, args).await,
        "hi_image_to_image" => return do_image_to_image(data_dir, args).await,
        "hi_text_to_video" => return do_text_to_video(data_dir, session_id, args).await,
        "hi_image_to_video" => return do_image_to_video(data_dir, session_id, args).await,
        "hi_review_view" => return do_review_view(data_dir, args).await,
        _ => {}
    }

    let arg_str =
        |key: &str| args.get(key).and_then(Value::as_str).unwrap_or_default().to_string();
    let arg_opt = |key: &str| args.get(key).and_then(Value::as_str).map(str::to_owned);

    // The switchboard calls, handled **before** any conversation is looked up.
    //
    // They reach the process-wide session registry and touch no conversation at all, so
    // requiring a live reaction loop to make one was a category error — and a load-bearing
    // one: `hi_create_worker` belongs to the standing rungs, and Reflection runs under a
    // sentinel conversation that has no loop, so the one rung holding the tool was the one rung
    // that could never call it. Same for the one verb: a standing agent could be sent
    // to, but could not send.
    match name {
        "hi_send_message" => {
            let Some(from) = session_id else {
                return tool_error("hi_send_message needs a session identity; this session has none");
            };
            let to = arg_str("to");
            let message = arg_str("message");
            if to.trim().is_empty() || message.trim().is_empty() {
                return tool_error("hi_send_message requires `to` and a non-empty `message`");
            }
            let Ok(target) = to.trim().parse::<registry::SessionId>() else {
                return tool_error(&format!(
                    "`{}` is not a session id. An address is a name — `cognition`, \
                     `reaction`, `reflection`, or a worker's, which comes back from \
                     `hi_create_worker`. Everyone you can reach is listed in your window \
                     with theirs.",
                    to.trim()
                ));
            };
            let delivery = registry::global().send(&from, &target, message.clone());

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
        "hi_session_status" => {
            let Some(id) = arg_str("id").trim().parse::<registry::SessionId>().ok() else {
                return tool_error("hi_session_status requires a session `id`");
            };
            let Some(st) = registry::global().status(&id) else {
                return tool_error(&format!("no live session {id}"));
            };
            let state = if st.busy {
                "working right now"
            } else if st.queued {
                "idle, with mail waiting"
            } else {
                "idle"
            };
            // `doing` is appended because `task` is what the worker was *given* and does not
            // change, so an owner polling this got the same sentence back whether the worker
            // was mid-command or wedged. That is the agent-facing half of the same blindness
            // the roster had — and the reason a watch could be reported as healthy while
            // nothing was running.
            let doing = match &st.doing {
                Some(what) => format!("; last seen doing: {what}"),
                None => "; nothing observed yet".to_string(),
            };
            return tool_ok(&format!(
                "session {} — {state}; {} turn(s) so far; on: {}{doing}",
                st.id, st.turns, st.title
            ));
        }
        "hi_close_worker" => {
            // The same three guards `hi_cancel_worker` carries, and for the same reasons:
            // lifetime is dispatch, dispatch belongs to the standing rungs, and a session
            // answers to whoever asked for the work.
            if !matches!(role, Some("reflection") | Some("cognition")) {
                return tool_error(&format!(
                    "`hi_close_worker` belongs to the standing rungs; role `{}` may not end \
                     a working session",
                    role.unwrap_or("<none>")
                ));
            }
            let Some(caller) = session_id else {
                return tool_error("hi_close_worker needs a session identity; this session has none");
            };
            let Some(id) = arg_str("id").trim().parse::<registry::SessionId>().ok() else {
                return tool_error("hi_close_worker requires a session `id`");
            };
            let Some(st) = registry::global().status(&id) else {
                return tool_ok(&format!(
                    "session {id} was already gone — nothing to close, and nothing is \
                     still holding its context."
                ));
            };
            if st.owner != Some(caller) {
                return tool_error(
                    "a working session can only be closed by the session that asked for the work",
                );
            }
            let owner_role = ToolOwner::from_role(role).expect("role guard above");
            let Some(sink) = registry.get(owner_role).await else {
                return tool_error("the owning loop is not up, so nothing can be closed");
            };
            // Waits for the answer, like `hi_cancel_worker`: whether a session is still
            // running is exactly the thing the caller is deciding about, so a hopeful
            // "closed" would put a session on the roster that is not there — or take one
            // off that is.
            let (reply, answer) = tokio::sync::oneshot::channel();
            if let Err(err) = sink.send(LoopControl::CloseWorker { id: id.clone(), reply }).await {
                return tool_error(&err.to_string());
            }
            return match tokio::time::timeout(std::time::Duration::from_secs(10), answer).await {
                Ok(Ok(true)) => tool_ok(&format!(
                    "session {id} is closed. If it was mid-turn it will finish and report \
                     once more, then end. Its context is gone — a further errand needs a \
                     new session."
                )),
                Ok(Ok(false)) => tool_ok(&format!(
                    "session {id} was already gone — nothing to close."
                )),
                Ok(Err(_)) => tool_error(&format!(
                    "the owning loop dropped the request; session {id} was not closed"
                )),
                Err(_) => tool_error(&format!(
                    "no answer from the owning loop in time; it is not confirmed that \
                     session {id} closed — check hi_session_status before assuming it is gone"
                )),
            };
        }
        "hi_cancel_worker" => {
            // Same rung guard as `hi_create_worker`, for the same reason: stopping work is
            // dispatch, and dispatch is the standing rungs'.
            if !matches!(role, Some("reflection") | Some("cognition")) {
                return tool_error(&format!(
                    "`hi_cancel_worker` belongs to the standing rungs; role `{}` may not \
                     stop work",
                    role.unwrap_or("<none>")
                ));
            }
            let Some(caller) = session_id else {
                return tool_error("hi_cancel_worker needs a session identity; this session has none");
            };
            let Some(id) = arg_str("id").trim().parse::<registry::SessionId>().ok() else {
                return tool_error("hi_cancel_worker requires a session `id`");
            };
            let Some(st) = registry::global().status(&id) else {
                return tool_error(&format!(
                    "no live session {id} — it may have already finished. Nothing was stopped."
                ));
            };
            // You may stop your own work and nobody else's. The switchboard already
            // holds the authoritative owner, so this reads the same fact `hi_send_message`
            // routes on rather than inventing a second answer to it.
            if st.owner != Some(caller) {
                return tool_error(
                    "a working session can only be stopped by the session that asked for the work",
                );
            }
            let owner_role = ToolOwner::from_role(role).expect("role guard above");
            let Some(sink) = registry.get(owner_role).await else {
                return tool_error("the owning loop is not up, so nothing can be stopped");
            };
            // **This one waits for its answer**, where `hi_create_worker` does not, because
            // the answer is the whole point: "I stopped it" and "it had already finished"
            // lead to different next moves and different things said to the person. The
            // loop serves its control channel *during* a turn, so the wait is short; the
            // timeout exists only so a wedged loop degrades to an honest "couldn't tell"
            // rather than hanging the caller mid-thought.
            let (reply, answer) = tokio::sync::oneshot::channel();
            if let Err(err) = sink.send(LoopControl::CancelWorker { id: id.clone(), reply }).await {
                return tool_error(&err.to_string());
            }
            return match tokio::time::timeout(std::time::Duration::from_secs(10), answer).await {
                Ok(Ok(true)) => tool_ok(&format!(
                    "stopped session {id} mid-work. It will report back with whatever it \
                     had got to. It stays alive and keeps its context, so hi_send_message it \
                     a new instruction if you want it to do something else instead."
                )),
                // Nothing was running. Said plainly, because the tempting summary —
                // "stopped" — would have the caller waiting on a report that is never
                // coming, and telling the person work was called off when it had in fact
                // already been done.
                Ok(Ok(false)) => tool_ok(&format!(
                    "nothing to stop — session {id} was not working when this arrived, so \
                     it had already finished or was already idle. No report is coming, and \
                     whatever it did is done. Check hi_session_messages if you need to know \
                     what that was."
                )),
                Ok(Err(_)) => tool_error(&format!(
                    "the owning loop dropped the request; session {id} was not stopped"
                )),
                Err(_) => tool_error(&format!(
                    "no answer from the owning loop in time; it is not confirmed that \
                     session {id} stopped — check hi_session_status before telling anyone it did"
                )),
            };
        }
        "hi_session_messages" => {
            let Some(id) = arg_str("id").trim().parse::<registry::SessionId>().ok() else {
                return tool_error("hi_session_messages requires a session `id`");
            };
            return match registry::global().messages(&id) {
                Some(text) if !text.trim().is_empty() => tool_ok(&text),
                Some(_) => tool_ok("that session has not said anything yet"),
                None => tool_error(&format!("no live session {id}")),
            };
        }
        "hi_create_worker" => {
            // Workers belong to the standing rungs — Cognition and Reflection, per
            // `docs/arch/foundation.md`. Reaction speaks and does not dispatch; a
            // conversation-bound rung that could would be a second dispatcher
            // (`docs/arch/agents.md`: "one dispatcher is the point").
            //
            // Structural, not just absent from the advertised surface — the same reason
            // `hi_say`/`hi_show` are checked above. Until now this was enforced only by
            // accident: Reaction had no `X-HI-Session-Id`, so the identity check below
            // rejected it. That fence is gone as of this commit, so the real one goes in.
            if !matches!(role, Some("reflection") | Some("cognition")) {
                return tool_error(&format!(
                    "`hi_create_worker` belongs to the standing rungs; role `{}` may not dispatch work",
                    role.unwrap_or("<none>")
                ));
            }
            let Some(owner) = session_id else {
                return tool_error("hi_create_worker needs a session identity; this session has none");
            };
            let task = arg_str("task");
            if task.trim().is_empty() {
                return tool_error("hi_create_worker requires a non-empty `task`");
            }
            // **Refused rather than derived, because a derived one is the bug.** The switchboard
            // used to register a worker under its brief, so every roster card, status line and
            // resume offer showed the first clause of a paragraph — which is setup, never the
            // subject. Cutting the brief here would rebuild exactly that. The caller is the one
            // party that knows what the errand *is* in five words, so it writes them or the
            // call does not go through; a retry costs a round-trip, an unreadable roster costs
            // the person every time they look at it.
            let title = arg_str("title");
            if title.trim().is_empty() {
                return tool_error(
                    "hi_create_worker requires a one-line `title` — what this errand is, in the \
                     words you would use to a colleague. It is what the person sees on their \
                     screen; the brief goes in `task`.",
                );
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
            // **Only a thread the boot glance actually offered.** A resume is the one
            // argument here whose value the model cannot derive from the task in front of
            // it — it has to come off the offer — and a thread id is exactly the shape of
            // thing that gets confabulated. Left unchecked it would still "work": the agent
            // layer falls back to a cold open when a resume fails, so an invented id would
            // produce a fresh session the caller believes carries context it has never
            // seen. Refusing is the difference between resuming and being told you did.
            let resume = match args.get("resume").and_then(|v| v.as_str()) {
                None => None,
                Some(thread) if thread.trim().is_empty() => None,
                Some(thread) => {
                    let offered = registry::global().lost_workers();
                    if !offered.iter().any(|end| end.thread.as_deref() == Some(thread)) {
                        return tool_error(&format!(
                            "no errand from the last run is on thread `{thread}` — `resume` \
                             takes a thread from the boot glance's offer, and there {}",
                            match offered.len() {
                                0 => "is no offer this run".to_string(),
                                n => format!("are {n} on it"),
                            }
                        ));
                    }
                    Some(thread.to_string())
                }
            };
            // A worker must run in the standing loop that created it. Role-specific
            // slots keep Cognition and Reflection from replacing each other's route.
            let owner_role = ToolOwner::from_role(role).expect("role guard above");
            let Some(sink) = registry.get(owner_role).await else {
                return tool_error(
                    "the owning loop is not up, so there is nowhere to run a worker",
                );
            };
            // Not validated against the ledger. A subject that names no task is a mislabelled
            // worker, which is visible and fixable; refusing the call over it would mean a
            // typo costs the work rather than the label. The projection simply finds no live
            // worker for that task, which is the same answer as not setting it at all.
            let subject = args
                .get("subject")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string);
            // The id is minted **here**, before the session exists, and handed back in
            // this reply — the contract is `CreateWorker → a session id`, and a caller
            // that cannot name what it made cannot brief it, ask after it, or read it.
            // Minting early is the same trick the whole session layer already uses: the
            // tool surface identifies its caller by a header, so an id has to exist
            // before the protocol assigns one.
            //
            // It is minted *after* `subject`, not before, because the slug is built from
            // it: `view-builder-kyoto-trip` says which errand this is, where an ordinal
            // said only how many had come before. The ledger subject is preferred over the
            // title because it is the name the task already has; the title is the fallback
            // for an errand the ledger does not carry.
            let id = registry::mint(
                Role::Worker(kind),
                Some(subject.as_deref().unwrap_or(title.as_str())),
            );
            let resumed = resume.is_some();
            return match sink
                .send(LoopControl::CreateWorker {
                    id: id.clone(),
                    title,
                    task,
                    kind,
                    owner: Some(owner),
                    resume,
                    subject,
                })
                .await
            {
                Ok(()) if resumed => tool_ok(&format!(
                    "session {id} starting from the errand's own thread — it opens knowing \
                     what that session knew, so brief it on what has *changed*, not on the \
                     job from scratch"
                )),
                Ok(()) => tool_ok(&format!(
                    "session {id} starting; brief it with hi_send_message, check it with \
                     hi_session_status"
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
        "hi_say" => {
            let text = arg_str("text");
            if text.trim().is_empty() {
                return tool_error("say requires non-empty `text`");
            }
            // The ack is what actually happened on each channel, not a constant: the
            // tool's whole justification is that speech is answerable, and an answer
            // that always reads "spoken" answers nothing. It also confirms the check-in
            // this call armed, so a promise the host is now holding is never something
            // the voice has to assume it made.
            sink.say(text, arg_opt("back_in").as_deref())
                .await
                .map(crate::body::reaction::Said::ack)
        }
        "hi_show" => {
            let op = args.get("op").and_then(Value::as_str).unwrap_or("show").to_string();
            // A view is normally shown by ref (one a worker built); resolve it to
            // source HERE, server-side, so the JSX never enters the mind's context.
            // Inline `source` stays as a trivial-one-off escape hatch. The ref may
            // carry a `.geom.json` sidecar — what the builder declared about it.
            // There is no placement to override here any more: views are full-bleed,
            // one at a time, so the mind decides *what* is on screen and never where.
            // The ref travels on with the view: it is the view's durable name, and
            // the compiled module URL it resolves to is a disposable content hash
            // that goes stale the moment the source is edited or the binary reseeds
            // `factory/`. Restoring the screen after a restart needs the name.
            let (view_ref, source, traits) = match arg_opt("ref") {
                Some(r) if !r.trim().is_empty() => {
                    match crate::mind::views::resolve_ref(data_dir, &r).await {
                        Ok((source, traits)) => (Some(r.trim().to_string()), source, traits),
                        Err(err) => return tool_error(&format!("show ref `{r}`: {err}")),
                    }
                }
                _ => (None, arg_str("source"), None),
            };
            sink.show(arg_opt("id"), op, source, traits, view_ref)
                .await
                .map(|()| "shown".to_string())
        }
        other => return tool_error(&format!("unknown tool: {other}")),
    };

    match outcome {
        Ok(ack) => tool_ok(&ack),
        Err(err) => tool_error(&err.to_string()),
    }
}

/// `hi_look`: capture the screen so the calling session can see where to act. Returns
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

/// `hi_review_view`: compile the saved view, render it in a real browser, and answer with
/// the verdict, the page's problems, and the screenshot.
///
/// The three steps are the same ones the build path takes, in the same order, which is
/// why this is a thin call rather than a second renderer: resolve the ref to source
/// (the compiler's existing job), compile it to a served module URL, then hand that to
/// [`view_render::render`], which owns the browser, the viewport policy and the blank
/// detection.
///
/// **The review frame IS the stage frame** — full-bleed, the only frame there is — so
/// a review renders the thing exactly the way `hi_show` will put it up. This used to be a
/// negotiation between the caller's override and the sidecar's declared region, and
/// getting it wrong failed a view for a defect the review itself introduced.
async fn do_review_view(data_dir: &std::path::Path, args: &Value) -> Value {
    let view_ref = args.get("ref").and_then(Value::as_str).unwrap_or_default().trim().to_string();
    if view_ref.is_empty() {
        return tool_error("hi_review_view requires a `ref`");
    }
    let (source, traits) = match crate::mind::views::resolve_ref(data_dir, &view_ref).await {
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
        .with_conversation(traits.is_some_and(|t| t.owns_conversation));

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
    // "Nothing is broken" used to be the whole sentence, and a builder answered it and
    // shipped: its own note back was "the first render is clean and readable", for a page
    // whose body was 12px mono. So the good/bad half now arrives as questions rather than
    // as an adjective — the checkable ones, since those are what a screenshot can settle.
    // Contrast still isn't scored here: the eye that catches "the names went invisible" is
    // the one looking at both frames, not a threshold.
    const FLOORS: &str = "body 16px or larger, prose not in mono, the most important text \
                          not the smallest on the page, lines of 45–90 characters, and the \
                          background quiet under the words that matter";
    let summary = if !failures.is_empty() {
        format!("`{view_ref}` did not render properly — {}", failures.join("; "))
    } else if shots.len() > 1 {
        format!(
            "`{view_ref}` rendered in both skins — nothing is broken. That is the only \
             question this tool answers; whether anyone can comfortably read it is yours. \
             Look at the pictures against the floors ({FLOORS}), then compare the two \
             frames: anything that fades out, disappears or turns unreadable in one of them \
             is a colour that only works in the other."
        )
    } else {
        format!(
            "`{view_ref}` rendered — nothing is broken. That is the only question this tool \
             answers; whether anyone can comfortably read it is yours. Look at the picture \
             against the floors ({FLOORS})."
        )
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

/// `hi_act`: synthesize one input action on the host. Coordinates arrive as normalized
/// 0..1 fractions of the screen (what the model reasons about, looking at `hi_look`'s
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

/// Map an `hi_act` `key` string to a [`crate::body::capabilities::input::Key`]. Named keys
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

/// Map an `hi_act` `mods` array to modifiers, accepting common aliases. Unknown
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

/// `hi_record_episode`: file the first `count` unconsolidated signals as one episode
/// (see [`crate::mind::memory::episodes::record_episode`]).
/// Returns the episode ref for the session to cite when it updates a facet.
async fn reflection_record_episode(data_dir: &std::path::Path, args: &Value) -> Value {
    let Some(count) = args.get("count").and_then(Value::as_u64) else {
        return tool_error("hi_record_episode requires an integer `count` >= 1");
    };
    let gist = args.get("gist").and_then(Value::as_str).unwrap_or_default();
    if gist.trim().is_empty() {
        return tool_error("hi_record_episode requires a non-empty `gist`");
    }
    let title = args.get("title").and_then(Value::as_str).unwrap_or_default();
    if title.trim().is_empty() {
        return tool_error("hi_record_episode requires a non-empty `title`");
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

/// `hi_read_facet`: return the current understanding of a subject, or a note that
/// none exists yet, so the session regenerates from the old rather than blank.
async fn reflection_read_facet(data_dir: &std::path::Path, args: &Value) -> Value {
    let dim = args.get("dimension").and_then(Value::as_str).unwrap_or_default();
    let subject = args.get("subject").and_then(Value::as_str).unwrap_or_default();
    if dim.trim().is_empty() || subject.trim().is_empty() {
        return tool_error("hi_read_facet requires `dimension` and `subject`");
    }
    match crate::mind::memory::facets::read_facet(data_dir, dim, subject).await {
        Ok(Some(content)) => tool_ok(&content),
        Ok(None) => tool_ok("(no facet yet — this subject has no recorded understanding)"),
        Err(err) => tool_error(&err.to_string()),
    }
}

/// `hi_update_facet`: write the whole regenerated understanding of a subject (see
/// [`crate::mind::memory::facets::update_facet`]). Returns the `<dim>/<subject>` ref.
async fn reflection_update_facet(data_dir: &std::path::Path, args: &Value) -> Value {
    let dim = args.get("dimension").and_then(Value::as_str).unwrap_or_default();
    let subject = args.get("subject").and_then(Value::as_str).unwrap_or_default();
    let content = args.get("content").and_then(Value::as_str).unwrap_or_default();
    if dim.trim().is_empty() || subject.trim().is_empty() {
        return tool_error("hi_update_facet requires `dimension` and `subject`");
    }
    if content.trim().is_empty() {
        return tool_error("hi_update_facet requires non-empty `content`");
    }
    match crate::mind::memory::facets::update_facet(data_dir, dim, subject, content).await {
        Ok(refname) => tool_ok(&format!("updated facet {refname}")),
        Err(err) => tool_error(&err.to_string()),
    }
}

/// `hi_update_proactivity`: regenerate the whole `proactivity.md` — the learned
/// read on speaking up unprompted — from how the agent's own unprompted utterances
/// landed (see [`crate::mind::memory::proactivity::write`]). Whole-file, never a patch.
async fn reflection_update_proactivity(data_dir: &std::path::Path, args: &Value) -> Value {
    let content = args.get("content").and_then(Value::as_str).unwrap_or_default();
    if content.trim().is_empty() {
        return tool_error("hi_update_proactivity requires non-empty `content`");
    }
    match crate::mind::memory::proactivity::write(data_dir, content).await {
        Ok(()) => tool_ok("updated proactivity.md"),
        Err(err) => tool_error(&err.to_string()),
    }
}

/// `hi_record_reflex`: teach a quick-action reflex (see [`crate::body::reflex`]). Stores the
/// fill value and how to find its field so a later invoke types it with no model in
/// the loop. The value itself is never echoed back in the ack.
async fn reflex_record(data_dir: &std::path::Path, args: &Value) -> Value {
    let name = args.get("name").and_then(Value::as_str).unwrap_or_default();
    let value = args.get("value").and_then(Value::as_str).unwrap_or_default();
    let label_contains = args.get("label_contains").and_then(Value::as_str).unwrap_or_default();
    if name.trim().is_empty() {
        return tool_error("hi_record_reflex requires a non-empty `name`");
    }
    if value.trim().is_empty() {
        return tool_error("hi_record_reflex requires a non-empty `value`");
    }
    if label_contains.trim().is_empty() {
        return tool_error("hi_record_reflex requires a non-empty `label_contains`");
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
        return tool_error("hi_record_reflex `name` must contain a usable character");
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

/// `hi_name_person`: rename a person's cluster (face or voice) from its `id` (or
/// current key) to a learned `name` — the structural side of "we now know who
/// this is". Merges if the name already exists. See [`people_vectors::rename`].
async fn reflection_name_person(data_dir: &std::path::Path, args: &Value) -> Value {
    let id = args.get("id").and_then(Value::as_str).unwrap_or_default();
    let name = args.get("name").and_then(Value::as_str).unwrap_or_default();
    if id.trim().is_empty() || name.trim().is_empty() {
        return tool_error("hi_name_person requires `id` and `name`");
    }
    match people_vectors::rename(data_dir, id, name).await {
        Ok(()) => tool_ok(&format!("named {id} → people/{name}")),
        Err(err) => tool_error(&err.to_string()),
    }
}

/// `hi_merge_people`: fold the `from` cluster into `into` (same person, two keys —
/// across senses too, e.g. a voice id into an already-named face). See
/// [`people_vectors::rename`].
async fn reflection_merge_people(data_dir: &std::path::Path, args: &Value) -> Value {
    let from = args.get("from").and_then(Value::as_str).unwrap_or_default();
    let into = args.get("into").and_then(Value::as_str).unwrap_or_default();
    if from.trim().is_empty() || into.trim().is_empty() {
        return tool_error("hi_merge_people requires `from` and `into`");
    }
    match people_vectors::rename(data_dir, from, into).await {
        Ok(()) => tool_ok(&format!("merged people/{from} → people/{into}")),
        Err(err) => tool_error(&err.to_string()),
    }
}

/// `hi_keep_and_fade`: let a cold consolidated day's media fade to text, keeping the
/// spans the mind chose (see [`crate::mind::memory::decay::keep_and_fade`]). The safety
/// gate lives in the tool, so an attempt on an un-consolidated day comes back as a
/// tool error the session can read, not a panic.
async fn reflection_keep_and_fade(data_dir: &std::path::Path, args: &Value) -> Value {
    let Some(channel) = args.get("channel").and_then(Value::as_str) else {
        return tool_error("hi_keep_and_fade requires `channel` (audio|vision)");
    };
    let Ok(channel) = channel.parse::<crate::types::Channel>() else {
        return tool_error(&format!("hi_keep_and_fade: unknown channel {channel:?}"));
    };
    let date = args.get("date").and_then(Value::as_str).unwrap_or_default();
    if date.trim().is_empty() {
        return tool_error("hi_keep_and_fade requires `date` (YYYY-MM-DD)");
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
                    "hi_keep_and_fade: keep[{i}] needs RFC3339 `start` and `end`"
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

/// `hi_image_text_to_text`: understand a stored still. Resolves the `ref` (the `⟨ref: …⟩` from a
/// `📷 photo arrived` signal, or one surfaced to reflection) to its bytes, then hands
/// it to [`perceive_still`] — which the bundle routes either to the model's own eyes
/// (native vision) or through the vision capability (text-only model).
async fn do_image_text_to_text(data_dir: &Path, args: &Value) -> Value {
    let prompt = args.get("prompt").and_then(Value::as_str).unwrap_or_default();
    let Some(reff) = args.get("ref").and_then(Value::as_str).filter(|s| !s.trim().is_empty()) else {
        return tool_error(
            "hi_image_text_to_text needs `ref` — the ⟨ref: …⟩ from the image's signal, e.g. vision/2026-06-25/14/23-07.jpg (pass it whole, channel included)",
        );
    };
    // Resolve first, sniff second — the same path the generation tools take.
    //
    // This used to derive the type from `parse_ref` *before* resolving, which meant
    // only a channel ref could get through: an image the agent had just drawn was
    // reported as a malformed ref, so "look at what you made" did not work on
    // anything it made. Reading the bytes answers both questions at once, and answers
    // the type question better — an extension is a claim, the magic number is a fact.
    let (bytes, mime) = match read_ref(data_dir, "hi_image_text_to_text", reff.trim()).await {
        Ok(v) => v,
        Err(e) => return e,
    };
    perceive_still(data_dir, bytes, &mime, prompt).await
}

/// `hi_video_text_to_text`: understand a short span of the live camera. Reads the
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
            "no live camera to watch — `hi_video_text_to_text` reads the camera streaming right now; \
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

/// Pull a tail length out of a `hi_video_text_to_text` span like "last 20s" / "20 seconds" → 20.0.
/// `None` (no number) means "the whole recent stretch".
fn parse_last_secs(span: &str) -> Option<f64> {
    let digits: String = span
        .chars()
        .skip_while(|c| !c.is_ascii_digit())
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    digits.parse::<f64>().ok().filter(|n| *n > 0.0)
}

// ── generation ────────────────────────────────────────────────────────────────
//
// The four tasks that *make* something. What distinguishes them from every other
// tool here is that they produce bytes with nowhere to be, so each handler ends the
// same way: file the artifact in `drive/` — the tree that does not fade — and hand
// back the `⟨ref: …⟩` that addresses it. The ref is the whole point of persisting
// rather than returning base64: it is what `hi_image_to_image`, `hi_image_to_video`,
// `hi_image_text_to_text` and `hi_show` all take, so one generation composes with
// everything already built.

/// Read the semantic knobs off the tool arguments. Absent stays absent — an omitted
/// knob must reach the vendor as "you decide", never as a default we invented.
fn image_params(args: &Value) -> image_gen::ImageParams {
    let s = |k: &str| {
        args.get(k).and_then(Value::as_str).map(str::trim).filter(|v| !v.is_empty()).map(str::to_owned)
    };
    image_gen::ImageParams {
        model: s("model"),
        size: s("size"),
        quality: s("quality"),
        n: args.get("n").and_then(Value::as_u64).map(|n| n as u32),
        seed: args.get("seed").and_then(Value::as_i64),
        background: s("background"),
        output_format: s("output_format"),
        watermark: args.get("watermark").and_then(Value::as_bool),
    }
}

fn video_params(args: &Value) -> video_gen::VideoParams {
    let s = |k: &str| {
        args.get(k).and_then(Value::as_str).map(str::trim).filter(|v| !v.is_empty()).map(str::to_owned)
    };
    video_gen::VideoParams {
        model: s("model"),
        resolution: s("resolution"),
        ratio: s("ratio"),
        duration: args.get("duration").and_then(Value::as_u64).map(|n| n as u32),
        seed: args.get("seed").and_then(Value::as_i64),
        watermark: args.get("watermark").and_then(Value::as_bool),
    }
}

/// Read a `⟨ref: …⟩` argument to bytes plus a content type, from either root.
async fn read_ref(data_dir: &Path, task: &str, reff: &str) -> Result<(Bytes, String), Value> {
    let Some(path) = crate::mind::memory::media::resolve_ref(data_dir, reff).await else {
        return Err(tool_error(&format!(
            "{task}: no media at {reff} (a camera still may have faded; a drive path may be \
             mistyped — pass the ref whole, channel or `drive/` included)"
        )));
    };
    let bytes = match tokio::fs::read(&path).await {
        Ok(b) => Bytes::from(b),
        Err(e) => return Err(tool_error(&format!("{task}: reading {reff} failed: {e}"))),
    };
    // Sniffed, not taken from the extension: a `.jpg` that is really a PNG would be
    // rejected by the provider with a content-type error nobody could act on.
    let mime = image_gen::sniff_mime(&bytes);
    Ok((bytes, mime))
}

/// File generated stills in the drive and report their refs.
///
/// `slug` is the prompt, which becomes part of the filename — the tree stays legible
/// to a human scrolling it a year later, which an opaque id would not be.
async fn land_images(
    data_dir: &Path,
    task: &str,
    slug: &str,
    images: Vec<image_gen::GeneratedImage>,
) -> Value {
    let now = chrono::Utc::now();
    let mut lines = Vec::new();
    for image in images {
        let ext = image_gen::extension_for(&image.mime);
        let reff = match crate::mind::memory::media::store_artifact(
            data_dir, now, slug, ext, &image.bytes,
        )
        .await
        {
            Ok(r) => r,
            Err(e) => return tool_error(&format!("{task}: the image was made but not saved: {e}")),
        };
        let path = crate::mind::memory::media::resolve_ref(data_dir, &reff)
            .await
            .map(|p| p.display().to_string())
            .unwrap_or_default();
        lines.push(format!("⟨ref: {reff}⟩\n  file: {path}\n  url: /api/drive/file/{}", &reff[6..]));
    }
    tool_ok(&format!(
        "{}\n\nFiled in the drive, which does not fade. Pass a ref to `hi_image_to_image` to \
         change it, to `hi_image_text_to_text` to look at what you made, or report it to \
         whoever asked so they can put it on screen.",
        lines.join("\n")
    ))
}

async fn do_text_to_image(data_dir: &Path, args: &Value) -> Value {
    let Some(prompt) =
        args.get("prompt").and_then(Value::as_str).filter(|s| !s.trim().is_empty())
    else {
        return tool_error("hi_text_to_image needs `prompt` — say what the image should show");
    };
    match image_gen::text_to_image(prompt, &image_params(args)).await {
        Ok(images) => land_images(data_dir, "hi_text_to_image", prompt, images).await,
        Err(e) => tool_error(&format!("hi_text_to_image failed: {e}")),
    }
}

async fn do_image_to_image(data_dir: &Path, args: &Value) -> Value {
    let Some(reff) = args.get("ref").and_then(Value::as_str).filter(|s| !s.trim().is_empty())
    else {
        return tool_error(
            "hi_image_to_image needs `ref` — the ⟨ref: …⟩ of the image to work from (a camera \
             still, a handed file, or one you generated)",
        );
    };
    let Some(prompt) =
        args.get("prompt").and_then(Value::as_str).filter(|s| !s.trim().is_empty())
    else {
        return tool_error("hi_image_to_image needs `prompt` — say what to change");
    };
    let (bytes, mime) = match read_ref(data_dir, "hi_image_to_image", reff.trim()).await {
        Ok(v) => v,
        Err(e) => return e,
    };
    let source = image_gen::SourceImage::bytes(bytes, mime);
    match image_gen::image_to_image(&source, prompt, &image_params(args)).await {
        Ok(images) => land_images(data_dir, "hi_image_to_image", prompt, images).await,
        Err(e) => tool_error(&format!("hi_image_to_image failed: {e}")),
    }
}

/// How long to keep asking about a submitted clip, and how often.
///
/// Generation runs to minutes, so the poll starts patient and grows; the ceiling
/// exists because a task that has told us nothing for a quarter of an hour is a
/// result too — one the session should hear rather than wait out forever.
const VIDEO_POLL_FIRST: std::time::Duration = std::time::Duration::from_secs(5);
const VIDEO_POLL_MAX: std::time::Duration = std::time::Duration::from_secs(20);
const VIDEO_POLL_DEADLINE: std::time::Duration = std::time::Duration::from_secs(15 * 60);

/// Submit a clip and arrange for its arrival to reach the session that asked.
///
/// **The clip comes back as mail, not as a return value.** A tool call that blocked
/// for the minutes this takes would hold the session's turn open against a network
/// timeout it does not control. `Registry::post` is the host putting something in an
/// agent's inbox (`from: None`) — so the arrival wakes the worker exactly as any other
/// message would.
fn spawn_video_poller(
    data_dir: PathBuf,
    session_id: Option<crate::foundation::registry::SessionId>,
    handle: video_gen::VideoHandle,
    task: &'static str,
    slug: String,
) {
    let started = tokio::time::Instant::now();
    tokio::spawn(async move {
        let mut wait = VIDEO_POLL_FIRST;
        let outcome = loop {
            tokio::time::sleep(wait).await;
            wait = (wait * 2).min(VIDEO_POLL_MAX);

            match video_gen::poll(&handle).await {
                Ok(t) if t.status.is_terminal() => break Ok(t.status),
                Ok(_) => {}
                // A single failed poll is a network blip, not a failed generation.
                // Only the deadline ends the wait.
                Err(e) => tracing::warn!(error = %e, task = %handle.id, "video poll failed"),
            }
            if started.elapsed() > VIDEO_POLL_DEADLINE {
                break Err(format!(
                    "still not finished after {} minutes; it may yet land upstream",
                    VIDEO_POLL_DEADLINE.as_secs() / 60
                ));
            }
        };

        let message = match outcome {
            Ok(video_gen::VideoStatus::Succeeded { video_url, .. }) => {
                match land_clip(&data_dir, &video_url, &slug).await {
                    Ok(reff) => format!("The clip you asked {task} for is ready: ⟨ref: {reff}⟩"),
                    Err(e) => format!("The {task} clip finished but could not be saved: {e}"),
                }
            }
            Ok(video_gen::VideoStatus::Failed { message }) => {
                format!("The {task} clip failed: {message}")
            }
            Ok(other) => format!("The {task} clip ended as {other:?}"),
            Err(e) => format!("The {task} clip ({}) {e}", handle.id),
        };

        // No session to tell, or it has ended: the artifact is still on disk, and
        // saying so in the log beats pretending the work was never done.
        let Some(to) = session_id else {
            tracing::info!(outcome = %message, "video generation finished with nobody to tell");
            return;
        };
        if registry::global().post(&to, message.clone()) != registry::Delivery::Delivered {
            tracing::info!(session = %to, outcome = %message, "video generation outlived its session");
        }
    });
}

/// Download a finished clip into the drive. Prompt, because the vendor's `video_url`
/// expires roughly a day after it is issued.
async fn land_clip(data_dir: &Path, url: &str, slug: &str) -> anyhow::Result<String> {
    let bytes = video_gen::fetch(url).await?;
    let reff = crate::mind::memory::media::store_artifact(
        data_dir,
        chrono::Utc::now(),
        slug,
        "mp4",
        &bytes,
    )
    .await?;
    Ok(reff)
}

async fn do_text_to_video(data_dir: &Path, session_id: Option<crate::foundation::registry::SessionId>, args: &Value) -> Value {
    let Some(prompt) =
        args.get("prompt").and_then(Value::as_str).filter(|s| !s.trim().is_empty())
    else {
        return tool_error("hi_text_to_video needs `prompt` — say what the clip should show");
    };
    match video_gen::text_to_video(prompt, &video_params(args)).await {
        Ok(handle) => {
            let id = handle.id.clone();
            spawn_video_poller(
                data_dir.to_path_buf(),
                session_id,
                handle,
                "hi_text_to_video",
                prompt.to_string(),
            );
            tool_ok(&format!(
                "Generating ({id}). This runs for minutes; the clip will arrive as a message \
                 with its ⟨ref: …⟩ when it is done, so carry on with something else — there is \
                 nothing to poll and nothing to wait for."
            ))
        }
        Err(e) => tool_error(&format!("hi_text_to_video failed: {e}")),
    }
}

async fn do_image_to_video(data_dir: &Path, session_id: Option<crate::foundation::registry::SessionId>, args: &Value) -> Value {
    let Some(reff) = args.get("ref").and_then(Value::as_str).filter(|s| !s.trim().is_empty())
    else {
        return tool_error("hi_image_to_video needs `ref` — the ⟨ref: …⟩ of the still to animate");
    };
    let prompt = args.get("prompt").and_then(Value::as_str).unwrap_or_default();
    let (bytes, mime) = match read_ref(data_dir, "hi_image_to_video", reff.trim()).await {
        Ok(v) => v,
        Err(e) => return e,
    };
    let frame = video_gen::ImageRef::bytes(bytes, mime);
    match video_gen::image_to_video(&frame, prompt, &video_params(args)).await {
        Ok(handle) => {
            let id = handle.id.clone();
            let slug = if prompt.trim().is_empty() { "animated".to_string() } else { prompt.to_string() };
            spawn_video_poller(
                data_dir.to_path_buf(),
                session_id,
                handle,
                "hi_image_to_video",
                slug,
            );
            tool_ok(&format!(
                "Animating ({id}). This runs for minutes; the clip will arrive as a message \
                 with its ⟨ref: …⟩ when it is done."
            ))
        }
        Err(e) => tool_error(&format!("hi_image_to_video failed: {e}")),
    }
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

    /// **Every declared tool carries the `hi_` prefix, and the prefix is load-bearing.**
    /// A rung's prompt names its tools bare — "`hi_send_message` reaches any other part of
    /// yourself" — and the model resolves that bare name against a surface we do not own:
    /// codex hands `gpt-5.6-sol` a `collaboration` namespace holding its own
    /// `send_message`, `spawn_agent` and `wait_agent` for the sub-agent tree it keeps
    /// inside one thread, plus a developer message telling the model it is `/root` in a
    /// team of agents. While our verb was also called `send_message`, 28 inter-rung
    /// messages across all four roles reached codex's router instead of ours; it answered
    /// `live agent path `/root/2` not found` — a bare session id resolving as a relative
    /// task name over there — and nothing was delivered. Two-thirds of them were never
    /// re-sent in that turn.
    ///
    /// The prefix cannot be dropped for a tool that happens not to clash today: codex
    /// ships `computer_use` and `browser_use` as stable features, so a built-in reaching
    /// for `look` or `act` is the next collision, not a hypothetical one.
    #[test]
    fn every_declared_tool_is_prefixed_against_the_hosts_own_surface() {
        for role in [Some("reaction"), Some("cognition"), Some("reflection"), Some("worker")] {
            for name in names(role) {
                assert!(name.starts_with("hi_"), "{role:?} declares `{name}` unprefixed");
            }
        }
    }

    /// **Every verb an agent is told to call is named in full, in prose too.**
    ///
    /// The prefix on the *declaration* (above) stops the collision; it does nothing about
    /// the sentence that tells a rung to use the tool. Those sentences live in three
    /// places — the bundled prompts, the tool descriptions, and text the host assembles at
    /// runtime — and a bare verb in any of them is read by the model against a surface we
    /// do not own, where the agent runtime keeps its own `send_message` for the sub-agent
    /// tree it holds inside one thread.
    ///
    /// That is not hypothetical twice over. The reachable roster said "send with
    /// `send_message`" for a day after the rename, in the one block rebuilt into every
    /// rung's window every turn. Then the consolidation prompt was found saying
    /// `update_proactivity`, `keep_and_fade` and `image-text-to-text`, and a duty
    /// handler's opening brief saying `send_message` — four more, in text no prompt file
    /// contains, which is exactly why sweeping the `.md` files was not enough.
    ///
    /// So the check is over *rendered text*, not over source: each declared name minus its
    /// prefix must not appear backticked anywhere an agent reads.
    #[test]
    fn no_agent_facing_text_names_a_verb_without_its_prefix() {
        use crate::identity::Role;

        let bare: Vec<String> = [
            Some("reaction"), Some("cognition"), Some("reflection"), Some("worker"),
        ]
        .iter()
        .flat_map(|r| names(*r))
        .filter_map(|n| n.strip_prefix("hi_").map(|b| format!("`{b}`")))
        .collect();

        let mut texts: Vec<(String, String)> = Vec::new();
        for role in Role::ALL {
            texts.push((format!("prompt {}", role.prompt_name()), role.base().to_string()));
        }
        for role in [Some("reaction"), Some("cognition"), Some("reflection"), Some("worker")] {
            for t in tools_for_role(role) {
                let name = t["name"].as_str().unwrap_or("?").to_string();
                texts.push((format!("description of {name}"), t.to_string()));
            }
        }
        // The runtime-assembled blocks: the ones that escaped the prompt sweep.
        texts.push((
            "the reachable roster".into(),
            crate::foundation::registry::render_reachable(&[(
                "cognition — the shared brain".into(),
                "cognition".parse().unwrap(),
            )]),
        ));
        texts.push((
            "the consolidation prompt".into(),
            format!(
                "{}{}",
                crate::body::reaction::PROACTIVITY_HEADING,
                crate::body::reaction::CONSOLIDATION_TOOLS,
            ),
        ));
        texts.push((
            "a duty handler's brief".into(),
            crate::body::reaction::DUTY_BRIEF_TAIL.to_string(),
        ));

        for (where_, text) in texts {
            for b in &bare {
                assert!(
                    !text.contains(b.as_str()),
                    "{where_} names {b} without its `hi_` prefix"
                );
            }
        }
    }

    /// Reaction's whole surface, pinned. `hi_say` lived in the unreachable fallback arm
    /// for the entire life of the reaction/cognition split — defined, dispatchable, and
    /// advertised to nobody — so the voice fell back to plain message text. Nothing
    /// failed; it just quietly stopped being a call that returns.
    #[test]
    fn reaction_holds_say_and_show_and_nothing_else() {
        let mut got = names(Some("reaction"));
        got.sort();
        assert_eq!(
            got,
            vec!["hi_say".to_string(), "hi_send_message".to_string(), "hi_show".to_string()],
            "its two expression channels, plus the one verb that reaches another agent"
        );
    }

    /// The other half of "and nothing else": a worker must not be able to speak.
    /// The one verb has to be on every rung, or an agent is unreachable by design.
    #[test]
    fn every_role_can_send_a_message() {
        for role in [Some("reaction"), Some("worker"), Some("cognition"), Some("reflection")] {
            assert!(
                names(role).contains(&"hi_send_message".to_string()),
                "{role:?} must hold hi_send_message"
            );
        }
    }

    /// An unknown role gets **nothing**, and that is the point.
    ///
    /// This arm used to hold the legacy agentic reaction's kit — `hi_say`, `hi_show`,
    /// `hi_record_reflex`, and the understanding tools — with a comment saying no live role
    /// mapped here. A fallback that hands out someone else's surface turns a missing arm
    /// into a silently wrong one — which is exactly what happened when a rung was opened
    /// under a role string with no arm and picked up `hi_say` it could not use.
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
        for role in ["reaction", "worker", "cognition", "reflection"] {
            assert!(!names(Some(role)).is_empty(), "`{role}` fell through to the empty fallback");
        }
    }

    /// One dispatcher. A the voice that could create workers would be a second one,
    /// spawning against Cognition unseen.
    /// Cognition's whole surface, pinned. Before it had an arm it fell into the `_`
    /// legacy fallback, which handed it `hi_say` and `hi_show` — refused at dispatch — and
    /// **not** `hi_create_worker`, the one tool it exists to use. A rung with no arm is not
    /// a rung with defaults; it is a rung with someone else's.
    ///
    /// `hi_cancel_worker` sits beside `hi_create_worker` because dispatch is two verbs: a rung
    /// that can start work and not stop it can only ever be told to change its mind too
    /// late.
    ///
    /// `hi_close_worker` is the third, and its absence used to be filled by a clock. A rung
    /// that can start work and stop a turn but never *finish* with a session does not own
    /// the lifetime — something else does, on a timer, with no idea whether the errand was
    /// done. All three or none.
    #[test]
    fn cognition_holds_the_switchboard_and_nothing_else() {
        let mut got = names(Some("cognition"));
        got.sort();
        assert_eq!(
            got,
            vec![
                "hi_cancel_worker".to_string(),
                "hi_close_worker".to_string(),
                "hi_create_worker".to_string(),
                "hi_send_message".to_string(),
                "hi_session_messages".to_string(),
                "hi_session_status".to_string(),
            ],
            "it delegates rather than does, and it has no mouth"
        );
    }

    #[test]
    fn only_the_standing_rungs_create_workers() {
        assert!(names(Some("reflection")).contains(&"hi_create_worker".to_string()));
        assert!(names(Some("cognition")).contains(&"hi_create_worker".to_string()));
        for role in [Some("reaction"), Some("worker")] {
            assert!(
                !names(role).contains(&"hi_create_worker".to_string()),
                "{role:?} must not create workers"
            );
        }
    }

    #[test]
    fn no_other_role_can_speak() {
        for role in [Some("worker"), Some("reflection")] {
            assert!(!names(role).contains(&"hi_say".to_string()), "{role:?} must not hold say");
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
                "hi_image_text_to_text",
                "hi_image_to_image",
                "hi_image_to_video",
                "hi_text_to_image",
                "hi_text_to_video",
                "hi_video_text_to_text",
            ]
        );
    }

    /// Understanding is advertised where it was before the rename; generation goes to
    /// the rung that produces artifacts. Pinned so a stub landing on Reaction — the one
    /// surface that is a hard rail — fails here rather than in a conversation.
    #[test]
    fn generation_belongs_to_workers_and_reaction_stays_a_voice() {
        let worker = names(Some("worker"));
        for task in ["hi_text_to_image", "hi_image_to_image", "hi_text_to_video", "hi_image_to_video"] {
            assert!(worker.contains(&task.to_string()), "worker must hold `{task}`");
            for role in [Some("reaction"), Some("cognition")] {
                assert!(!names(role).contains(&task.to_string()), "{role:?} must not hold `{task}`");
            }
        }
        assert!(worker.contains(&"hi_video_text_to_text".to_string()));
        assert!(names(Some("reflection")).contains(&"hi_image_text_to_text".to_string()));
    }

    /// Every advertised tool must dispatch. A tool in the surface with no arm falls
    /// through to "unknown tool", which tells the model its call was malformed when
    /// the truth is something else entirely — and a wrong reason outlives a wrong
    /// result. With no provider configured in a test process, the four generation
    /// tasks must report *configuration*, never the fallback.
    #[tokio::test]
    async fn the_generation_tasks_dispatch_rather_than_falling_through() {
        let dir = tempfile::tempdir().unwrap();
        let tools = crate::body::reaction::ToolRegistry::new();
        let partial = Mutex::new(None);
        let obs = Observatory::new(None);

        for name in ["hi_text_to_image", "hi_image_to_image", "hi_text_to_video", "hi_image_to_video"] {
            let got = dispatch_tool(
                &tools,
                dir.path(),
                &partial,
                &obs,
                Some(7.into()),
                Some("worker"),
                name,
                &json!({ "prompt": "a cat", "ref": "2026-06-25/14/23-07.jpg" }),
            )
            .await;
            assert_eq!(got.get("isError").and_then(Value::as_bool), Some(true), "{name}");
            let text = got["content"][0]["text"].as_str().unwrap();
            assert!(!text.contains("unknown tool"), "{name} fell through to the fallback: {text}");
            assert!(text.contains(name), "{name} must name itself: {text}");
        }
    }

    /// Every argument is optional except the prompt, and an omitted knob must stay
    /// omitted all the way down — a `None` here is "the model decides", and turning it
    /// into a default is us deciding while reporting that the model did.
    #[test]
    fn omitted_knobs_do_not_become_defaults() {
        let bare = image_params(&json!({ "prompt": "a cat" }));
        assert!(bare.model.is_none() && bare.size.is_none() && bare.n.is_none());
        assert!(bare.quality.is_none() && bare.background.is_none() && bare.seed.is_none());

        // Blank strings are omissions too: a model that fills a field with "" has not
        // chosen a size, and passing it on turns that into a vendor error.
        let blank = image_params(&json!({ "prompt": "a cat", "model": "  ", "size": "" }));
        assert!(blank.model.is_none() && blank.size.is_none());

        let set = image_params(&json!({ "model": "gpt-image-2", "n": 2, "seed": 7 }));
        assert_eq!(set.model.as_deref(), Some("gpt-image-2"));
        assert_eq!(set.n, Some(2));
        assert_eq!(set.seed, Some(7));

        let v = video_params(&json!({ "duration": 5, "ratio": "16:9" }));
        assert_eq!(v.duration, Some(5));
        assert_eq!(v.ratio.as_deref(), Some("16:9"));
        assert!(v.model.is_none() && v.resolution.is_none());
    }

    /// The menu is the whole of what makes "you choose the model" a real instruction.
    /// An empty one must say so plainly rather than leaving the agent to guess whether
    /// silence means "no models" or "any model".
    #[test]
    fn the_model_property_describes_what_is_reachable() {
        let menu = vec![
            MenuEntry { name: "gpt-image-2".into(), quality: 90, speed: 40, price: 30 },
            MenuEntry { name: "gpt-image-1-mini".into(), quality: 50, speed: 90, price: 5 },
        ];
        let prop = model_property(menu, Some("gpt-image-2".into()), "draw");
        let d = prop["description"].as_str().unwrap();
        assert!(d.contains("gpt-image-2 (highest quality)"), "{d}");
        assert!(d.contains("gpt-image-1-mini (fastest, cheapest)"), "{d}");
        assert!(d.contains("Omit to use gpt-image-2"), "{d}");

        let empty = model_property(Vec::new(), None, "draw");
        let d = empty["description"].as_str().unwrap();
        assert!(d.contains("passed through as given"), "{d}");

        // One model is not "highest quality, fastest, cheapest" — it is the only one.
        let solo = vec![MenuEntry { name: "seedream".into(), quality: 9, speed: 9, price: 9 }];
        let d = model_property(solo, None, "draw")["description"].as_str().unwrap().to_string();
        assert!(d.contains("Reachable now: seedream."), "{d}");
        assert!(!d.contains("highest quality"), "{d}");
    }

    /// Surface membership is a context optimization, not a rail — so the rungs that
    /// must not dispatch are refused at *dispatch*, whatever their model emits.
    ///
    /// This was enforced only by accident until Reaction was given a session id: the
    /// identity check rejected it for having no `X-HI-Session-Id`, which reads as a
    /// guard and is not one. A rung with an identity and no advertised `hi_create_worker`
    /// could call it anyway.
    #[tokio::test]
    async fn create_worker_is_refused_to_non_owner_roles_at_dispatch() {
        let dir = tempfile::tempdir().unwrap();
        let tools = crate::body::reaction::ToolRegistry::new();
        let partial = Mutex::new(None);
        let obs = Observatory::new(None);

        for role in [Some("reaction"), Some("worker"), None] {
            let got = dispatch_tool(
                &tools,
                dir.path(),
                &partial,
                &obs,
                // An identity, so this cannot pass for the old accidental rejection.
                Some(7.into()),
                role,
                "hi_create_worker",
                &json!({ "task": "do a thing" }),
            )
            .await;
            assert_eq!(got.get("isError").and_then(Value::as_bool), Some(true), "{role:?}");
            let text = got["content"][0]["text"].as_str().unwrap();
            assert!(text.contains("may not dispatch work"), "{role:?} got: {text}");
        }
    }

    /// **A brief with no title is refused, not summarized.** The switchboard registers the
    /// title and every reader of it renders one line, so a call that leaves the title out
    /// leaves the host two options: cut the brief, or ask. Cutting is what this whole pair
    /// of arguments exists to stop — a paragraph's first clause is setup, never the subject
    /// — so the call does not go through, and the error says what to write.
    #[tokio::test]
    async fn create_worker_refuses_a_brief_with_no_title() {
        let dir = tempfile::tempdir().unwrap();
        let tools = crate::body::reaction::ToolRegistry::new();
        let partial = Mutex::new(None);
        let obs = Observatory::new(None);

        for args in [
            json!({ "task": "Deploy only hi-agent.xyz end to end. First read the ledger…" }),
            json!({ "task": "do a thing", "title": "   " }),
        ] {
            let got = dispatch_tool(
                &tools,
                dir.path(),
                &partial,
                &obs,
                Some(7.into()),
                Some("cognition"),
                "hi_create_worker",
                &args,
            )
            .await;
            assert_eq!(got.get("isError").and_then(Value::as_bool), Some(true), "{args}");
            let text = got["content"][0]["text"].as_str().unwrap();
            assert!(text.contains("one-line `title`"), "got: {text}");
        }

        // And the schema says so too, so the model is told before it is refused.
        let schema = &create_worker_tool()["inputSchema"];
        assert_eq!(schema["required"], json!(["title", "task"]));
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
                Some(7.into()),
            Some("reaction"),
            "hi_send_message",
            &json!({ "to": "99", "message": "are you there" }),
        )
        .await;
        assert_eq!(got.get("isError").and_then(Value::as_bool), Some(true));

        let (replay, _rx) = obs.subscribe().await;
        assert_eq!(replay.len(), 1, "the failed edge is history too");
        let v = serde_json::to_value(&replay[0]).unwrap();
        assert_eq!(v["event"], "message_sent");
        assert_eq!(v["from"], "7");
        assert_eq!(v["to"], "99");
        assert_eq!(v["delivery"], "unknown");
        assert_eq!(v["message"], "are you there");
    }
}
