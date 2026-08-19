//! hi-agent — reference implementation of the human-interface spec.

use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use tokio::net::TcpListener;
use tokio::sync::{Notify, watch};

pub mod appearance;
pub mod body;
pub mod bundle;
pub mod foundation;
pub mod identity;
pub mod mind;
pub mod net;
pub mod runtime;
pub mod types;

#[derive(Debug, Clone)]
pub struct Config {
    /// The loopback port. Everything on this machine talks to the core here —
    /// the face in dev, the codex subprocesses on `/mcp`, `HI_AGENT_BASE_URL`,
    /// the headless renderer — and none of it is gated.
    pub port: u16,
    /// Where to accept **off-box** requests, if anywhere. Unset by default: a
    /// desktop install is reached over loopback and needs no open socket at all.
    /// Set it for the directly-public shape (a core in Docker behind a domain),
    /// and everything arriving here is gated. It cannot share [`Self::port`] —
    /// one socket cannot tell loopback from the world, and that distinction is
    /// the whole of the trust model (`docs/arch/topology.md`, invariant 6).
    pub off_box: Option<std::net::SocketAddr>,
    /// Where the **app** listens, if this install is one. The face talks only
    /// here; the app forwards to whichever core its roster is attached to and
    /// adds the credential on the way out. Unset on a headless core (Docker),
    /// which is a core and nothing else. See [`app`].
    pub app_port: Option<u16>,
    pub data_dir: PathBuf,
    pub agent: foundation::config::AgentConfig,
    pub auth: foundation::auth::AuthConfig,
}

/// Absolutize `dir` against the current working directory (if relative) and
/// lexically strip `.`/`..` components so it reads as a clean absolute path.
/// Purely lexical — it does not touch the filesystem or resolve symlinks.
fn normalize_dir(dir: &Path) -> anyhow::Result<PathBuf> {
    let abs = if dir.is_absolute() {
        dir.to_path_buf()
    } else {
        std::env::current_dir()?.join(dir)
    };
    let mut out = PathBuf::new();
    for comp in abs.components() {
        match comp {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    Ok(out)
}

/// Public entry: serve until SIGINT/SIGTERM. Thin wrapper over
/// [`run_with_shutdown`] with a trigger that never fires, so the only shutdown
/// sources are the OS signals — byte-for-byte the historical behavior. The Linux,
/// Docker, and headless-macOS paths all enter here.
pub async fn run(config: Config) -> anyhow::Result<()> {
    run_with_shutdown(config, Arc::new(Notify::new())).await
}

/// Build the axum app, spawn the codex subprocess + reaction, bind, and serve until
/// the process is terminated by an OS signal **or** `shutdown` is notified. The
/// notify is the macOS tray's "Quit" path ([`run_with_tray`]); everywhere else it
/// is a no-op trigger handed in by [`run`].
async fn run_with_shutdown(config: Config, shutdown: Arc<Notify>) -> anyhow::Result<()> {
    // Normalize the data dir once, up front: absolutize it (it rides to child
    // processes via env, which may run with a different cwd) and strip `.`/`..`
    // components so the paths we hand the mind read as clean absolutes —
    // `.../hi-agent/data/prompts/cognition.md`, not `.../hi-agent/./data/prompts/…`.
    // Every downstream consumer (prompts_dir, views_dir, the prompt placeholders, …)
    // inherits this.
    let mut config = config;
    config.data_dir =
        normalize_dir(&config.data_dir).context("resolving cwd to absolutize data dir")?;
    tracing::debug!(?config, "starting hi-agent");

    // Snapshot the cognition tunables (pulse, reflection cadence, compact ceiling,
    // vendor-down thresholds, …) and the declared owner from the config store into the
    // process global the reaction's argless helpers read. Once, before anything reads
    // them — and for the owner that ordering is load-bearing rather than tidy: every
    // addressed signal is attributed at the instant it arrives, so this must be in
    // place before the server can bind.
    foundation::config::tunables::init(&config.data_dir);

    // Restore an observed managed 402 before asking the broker for fresh account
    // state. A positive refresh clears it; an unreachable broker leaves the gate
    // paused instead of letting a restart erase the condition.
    foundation::energy_state::restore(&config.data_dir);

    // Xiaoyuanzhu: refresh the managed credential bundle from the broker, then
    // re-resolve the LLM credential from the (possibly updated) store —
    // `build_config` resolved it before this point, and BYOK mode is a no-op so
    // this changes nothing there. Best-effort: a broker failure leaves the cached
    // bundle (or boots unconfigured). No request context here, so no Authentik
    // token — a signed-in `sub` upgrade only happens on mode-select in Settings,
    // where the session token is available; startup mints/keeps the device account.
    foundation::broker::refresh(&config.data_dir, None).await;
    config.agent = foundation::config::AgentConfig::resolve(&config.data_dir);

    // Keep managed credentials fresh while running: re-fetch configs on a slow
    // cadence (rotating the access token) and poll energy on a fast one. No-op in
    // BYOK. New sessions pick up a rotated token; long-running ones on respawn.
    foundation::broker::spawn_refresh_loop(config.data_dir.clone());

    let memory = mind::memory::Memory::open(&config.data_dir).await?;
    tracing::info!(data_dir = %config.data_dir.display(), "memory opened");

    // Materialise the bundled prompts under <data_dir>/prompts/ — one whole prompt per
    // rung, plus workers/<type>.md — so each is on
    // disk. Absolutize the dir:
    // it rides to the child as HI_AGENT_PROMPTS_DIR, and the child may run with a
    // different cwd than us.
    identity::install_prompts(&config.data_dir).context("installing bundled prompts")?;
    // Before anything reads a seed: bring forward the ones written under `memory/` by a
    // build that filed them as memory rather than as what a session is handed.
    mind::memory::layout::migrate_seeds(&config.data_dir);
    let prompts_dir = {
        let d = config.data_dir.join("prompts");
        if d.is_absolute() {
            d
        } else {
            std::env::current_dir()
                .context("resolving cwd to absolutize prompts dir")?
                .join(d)
        }
    };

    // The agent's view workshop — the disposable tree where views are built. It's
    // every worker's cwd (so a build sub-agent works in a real project dir) and where
    // it writes view source (`<project>/<name>.jsx`). Absolutized as above; also the
    // root the server serves at `/views/*` (compiled modules land in `_compiled`).
    let views_dir = {
        let d = config.data_dir.join("views");
        if d.is_absolute() {
            d
        } else {
            std::env::current_dir()
                .context("resolving cwd to absolutize views dir")?
                .join(d)
        }
    };
    std::fs::create_dir_all(&views_dir).context("creating views dir")?;

    // Seed the bundled built-in views (the file-upload entry) into the tree so the
    // agent can show them by ref like any view. Overwritten each boot — the tree is
    // disposable, so a binary update reseeds the latest.
    mind::views::install_factory_views(&config.data_dir).context("installing built-in views")?;

    // The agent's skill workshop. Seeds the factory layer under `skills/factory/`
    // (rewritten each boot) and creates the tree, so the workshop exists before the
    // first note is written. Agent-written skills land alongside and are never touched.
    mind::skills::install_factory_skills(&config.data_dir).context("installing built-in skills")?;

    // The agent's precious drive — where it files artifacts worth keeping (a user's
    // handed-over documents, its own kept work). Created here so it always exists;
    // filling it is the agent's job. (Verbatim annex of memory; see data-dir-layout.)
    std::fs::create_dir_all(config.data_dir.join("drive")).context("creating drive dir")?;

    // Structured visibility into the session lifecycle. The agent layer,
    // reaction, workers and heartbeat feed it; `GET /api/sessions` reads the live
    // mirror and `GET /api/sessions/events` streams the history over SSE.
    let observatory =
        foundation::observatory::Observatory::new(Some(config.data_dir.join("sessions.jsonl")));

    // Raw wire tap — every JSON-RPC frame, business-logic agnostic. The agent layer
    // hands it to each rung's subprocess; `GET /api/wire/frames/events` streams it to
    // the raw session inspector.
    let wire_tap = foundation::codex::WireTap::with_durable_log(config.data_dir.clone());

    // The local/private side of the external-model boundary. This owns the private
    // secret store, the maintained detectors, a per-boot proxy token, and the HTTP
    // client used by both the model proxy and brokered API calls.
    let privacy = foundation::privacy::PrivacyBoundary::open(&config.data_dir)
        .context("initializing the private-data boundary")?;

    // The session directory — what ran and where its frames are. The tap above has always
    // kept the frames; nothing could address them once a session left the switchboard,
    // because session ids restart at 1 each run. Attaching this also seeds the
    // recent-ends list from previous runs, which is what makes a worker the process died
    // underneath visible instead of simply absent.
    foundation::registry::global().attach_index(config.data_dir.clone()).await;

    // Resolve all keyed capabilities BYOK-first: each vendor's key from the
    // credential store (`<data_dir>/credentials.json`) wins, else its `.env` key.
    // Unconfigured capabilities are fine; gates affect /audio (STT) and the speak
    // path (TTS) only.
    let creds = foundation::credentials::Credentials::load(&config.data_dir);
    body::capabilities::init(&creds)?;
    // Voice/face recognition need no env config — provision their pinned local
    // ONNX models on first run (cached thereafter) and load them. Best-effort:
    // a failed provision leaves the capability disabled, never blocks startup.
    body::capabilities::init_recognition().await;
    tracing::info!(
        stt = body::capabilities::stt::available(),
        tts = body::capabilities::tts::available(),
        voiceprint = body::capabilities::voiceprint::available(),
        face = body::capabilities::face::available(),
        "capabilities resolved"
    );

    // The tool-sink slot shared between the HTTP front's `/mcp` handler and the
    // reaction that registers its sink. The mind drives output and
    // side-effects by calling tools on `/mcp`; they route here.
    let tool_registry = body::reaction::ToolRegistry::new();
    // The floor, shared the same way: the server's STT relay reports recognized
    // speech, which both marks the floor theirs — `say` is refused while it is —
    // and is what a barge-in is inferred against. No cancel, no endpoint, and no
    // "they have stopped" endpoint either: a client cannot know that.
    let floor = body::reaction::Floor::new();
    // Live-subscriber counts, shared the same way: the server's out-channel
    // handlers hold a guard per connection. One question is asked of them — is a
    // speaker attached, so a TTS span is worth synthesizing.
    let attachments = body::attachments::Attachments::new();

    // Build the owner sign-in state (None when OIDC is unconfigured — sign-in
    // unavailable, free tier only). Fallible: it generates/reads the cookie key
    // under <data_dir>/auth/, so a bad key file surfaces here, not mid-request.
    let auth_state =
        foundation::auth::AuthState::from_config(config.auth.clone(), &config.data_dir)
            .context("initializing owner sign-in")?;

    let (router, seams) = foundation::server::build(
        memory.clone(),
        config.data_dir.clone(),
        observatory.clone(),
        wire_tap.clone(),
        privacy.clone(),
        tool_registry.clone(),
        floor.clone(),
        attachments.clone(),
        auth_state,
    );

    // Resolve the runtime: prefer system tools on PATH, else install on first run.
    let runtime = runtime::ensure().await?;
    tracing::info!(origin = runtime.origin, "runtime resolved");

    // Codex keeps its own state (logs, sqlite, caches) under CODEX_HOME. Give it one
    // inside our data dir rather than letting it write to `~/.codex`, so a hi-agent
    // install never collides with the user's own codex. Absolutized because child
    // processes may run with a different cwd than us.
    let codex_home = {
        let dir = config.data_dir.join("codex-home");
        if dir.is_absolute() {
            dir
        } else {
            std::env::current_dir()
                .context("resolving cwd to absolutize the codex home")?
                .join(dir)
        }
    };
    std::fs::create_dir_all(&codex_home).context("creating the codex home")?;
    if !config.agent.is_configured() {
        tracing::warn!(
            "no LLM credentials configured — the broker (xiaoyuanzhu) should mint them \
             automatically; otherwise set a key in Settings (BYOK). The agent boots but \
             prompts fail until a key is set"
        );
    }

    // Spawn config for the agent session layer. The subprocess itself is spawned
    // lazily, one per session (Chrome-style isolation); the pinned runtime and managed
    // env are shared by all. The child reaches only the loopback privacy proxy; the
    // trusted host resolves and injects the current upstream credential.
    let mut child_env = config.agent.child_env(config.port, &codex_home);
    // Sessions read their own prompt back from <prompts>/ at open; hand them the
    // absolute dir the same way workers already get HI_AGENT_BASE_URL.
    child_env.push((
        "HI_AGENT_PROMPTS_DIR".to_string(),
        prompts_dir.display().to_string(),
    ));
    tracing::info!(
        cwd = ?std::env::current_dir().ok(),
        runtime_origin = runtime.origin,
        codex_bin = %runtime.codex_bin.display(),
        codex_home = %codex_home.display(),
        model = ?config.agent.model,
        "child runtime and local model proxy resolved"
    );

    let agent = foundation::agent::AgentLayer::new(
        foundation::agent::SpawnConfig {
            program: runtime.codex_bin.clone(),
            args: vec!["app-server".to_string(), "--stdio".to_string()],
            env: child_env,
        },
        config.data_dir.clone(),
        wire_tap,
        format!("http://127.0.0.1:{}", config.port),
        privacy,
    );
    tracing::info!("agent session layer ready (one subprocess spawns per session)");
    // A handle for shutdown: the reaction takes ownership of `agent` below, but on
    // termination we still need to reap every subprocess it spawned. The clone
    // shares the same process registry.
    let agent_for_shutdown = agent.clone();

    // The reaction compiles view source to ESM via esbuild; modules land under
    // data_dir/views/_compiled. esbuild is hi-agent's own tool (not the
    // adapter's) — `ensure_view_esbuild` guarantees one whether the runtime came
    // from PATH or the managed install, so views aren't silently broken in dev.
    let esbuild_bin = runtime::ensure_view_esbuild(&runtime)
        .await
        .context("resolving esbuild for the view compiler")?;
    let view_compiler = mind::views::ViewCompiler::new(esbuild_bin, &config.data_dir);
    // The reviewing half needs the same compiler plus our own origin, and it runs from
    // a *tool call* rather than from the reaction — so it cannot be handed either down
    // the conversation path. Published here, read by `review_view`.
    mind::views::set_render_context(
        view_compiler.clone(),
        format!("http://127.0.0.1:{}", config.port),
    );
    // The screen came back from the last snapshot when the router was built, but a
    // snapshot pins the *compiled module* a view was shown as, and `install_factory_views`
    // above may have just reseeded that view's source from a newer binary. Recompile it
    // from source now — the first moment both the reseeded tree and a compiler exist —
    // so what's on screen is the view as it is today, not as it was when it was shown.
    seams.state.views.refresh_sources(&view_compiler).await;
    // The person's language, stamped onto `<html lang>` so a bundled view can pick which
    // of its copies to show. Captured once here for the same reason the setting says it
    // applies on restart. Unset reads as `system`, which the page resolves against the
    // machine — English if that matches nothing we ship.
    appearance::set_language(
        foundation::credentials::get_setting(&config.data_dir, foundation::config::KEY_LANGUAGE)
            .unwrap_or_else(|| "system".to_string()),
    );
    // The reaction's shutdown signal: triggered below the moment a signal / Quit is
    // observed, so its reaction loops, reflection, and drive retries wind down instead
    // of restarting agent sessions into a process group that's already terminating.
    let reaction_shutdown = foundation::shutdown::Shutdown::new();
    // Eager sessions attach to `/mcp` during `session/new`. The reaction starts before
    // the listener below, so retain one readiness edge that every startup warm-up can
    // await without racing the HTTP server.
    let (server_ready_tx, server_ready_rx) = watch::channel(false);
    let _reaction = body::reaction::start(
        memory,
        agent,
        seams.inbound_rx,
        seams.warm_rx,
        seams.duty_rx,
        seams.out_tx,
        observatory,
        view_compiler,
        tool_registry,
        floor,
        attachments,
        seams.state.views.clone(),
        views_dir,
        reaction_shutdown.clone(),
        server_ready_rx,
    )
    .await?;
    tracing::info!("reaction started");

    // Arm the "come and see this" gesture: a double-tap of Command hands the agent
    // a screenshot of the current screen as a file (macOS only, best-effort — needs
    // the Accessibility + Screen Recording grants, else it stays inert). It joins
    // the conversation like any other signal — the same one the browser is talking
    // in, which is the whole point: showing the agent your screen and then asking
    // about it out loud is one exchange.
    //
    // Off unless the user has opted in (the tray's "Attention gestures" item): the
    // global key event tap forces the macOS "Input Monitoring" grant the moment it's
    // created, and we don't want that prompt out of the box. Enabling the setting and
    // restarting arms it — and that's when the grant is requested.
    if foundation::config::flag_on(foundation::config::tunables::get(
        foundation::config::KEY_GESTURES,
    )) {
        body::gesture::install(seams.state.clone());
    } else {
        tracing::info!("attention gestures off (enable in the tray menu to arm them)");
    }

    // Two acceptors, and which one took a request is the whole trust decision —
    // never a header, never a source address (`docs/arch/topology.md`,
    // invariant 6). The loopback listener binds `127.0.0.1` *only*, so "arrived
    // here" is a fact about the socket that no sender can forge. Anything else is
    // off-box and gated, whether it came over the public bind or (later) as a
    // stream routed in over the community tunnel.
    //
    // This is why the two cannot share a port: a single `0.0.0.0` socket accepts
    // loopback and the LAN indistinguishably, which is exactly the question the
    // gate has to answer.
    let listener = TcpListener::bind(("127.0.0.1", config.port)).await?;
    tracing::info!("hi-agent listening on http://127.0.0.1:{} (loopback, ungated)", config.port);

    let off_box_listener = match config.off_box {
        Some(addr) => {
            let l = TcpListener::bind(addr)
                .await
                .with_context(|| format!("binding the off-box listener on {addr}"))?;
            tracing::info!(%addr, "hi-agent accepting off-box requests (gated)");
            Some(l)
        }
        None => {
            tracing::info!(
                "no off-box listener (set --off-box / HI_AGENT_OFF_BOX to be reachable \
                 from anywhere but this machine)"
            );
            None
        }
    };

    // Mint the bootstrap credential the first time this core runs, and say it out
    // loud exactly once. Without it a core with no screen and no paired app — the
    // Docker shape — could never admit its first surface. Only when the off-box
    // listener exists: on a loopback-only install nothing is gated, so a
    // credential nobody needs would be a secret printed for no reason.
    if off_box_listener.is_some() {
        if let Some(token) = seams.state.surfaces.ensure_first_boot_credential() {
            tracing::info!(
                credential = %token,
                "first-boot surface credential — pair with it once, then revoke it in Settings"
            );
        }
    }

    // The app, when this install is one: a loopback proxy in front of a roster.
    // Started after the core's listeners so its first entry — the core on this
    // machine — is reachable the moment it is written.
    let app_server = match config.app_port {
        Some(app_port) => {
            hi_app::roster::ensure_local(&config.data_dir, config.port)
                .context("seeding the roster with the core on this machine")?;
            let app = Arc::new(hi_app::App::new(config.data_dir.clone())?);
            let listener = TcpListener::bind(("127.0.0.1", app_port))
                .await
                .with_context(|| format!("binding the app on 127.0.0.1:{app_port}"))?;
            tracing::info!(
                "the app is at http://127.0.0.1:{app_port} — open this, not the core; \
                 its roster is at http://127.0.0.1:{app_port}/app"
            );
            let router = hi_app::proxy::router(app);
            let shutdown = shutdown.clone();
            Some(tokio::spawn(async move {
                let r = axum::serve(listener, router)
                    .with_graceful_shutdown(shutdown_requested(shutdown))
                    .await;
                if let Err(e) = r {
                    tracing::error!(error = %e, "the app's proxy stopped");
                }
            }))
        }
        None => None,
    };

    // Hold a tunnel open if this core has a name, so it is reachable by that name
    // from anywhere. Off the boot path: it dials the community, and a community
    // that is slow or down must not delay a core that works locally.
    {
        let data_dir = config.data_dir.clone();
        let router = router.clone();
        tokio::spawn(async move { foundation::tunnel::start(&data_dir, router).await });
    }

    // Record the bound port so the native Settings "Sign in" button and the
    // account-link handlers can build the loopback callback URL (not a secret).
    let _ = foundation::credentials::set_setting(
        &config.data_dir,
        foundation::credentials::KEY_SERVER_PORT,
        &config.port.to_string(),
    );

    // Serve until SIGINT/SIGTERM or the tray's Quit. `with_graceful_shutdown`
    // stops accepting new connections and lets in-flight requests finish. We run
    // it in a task so we can also watch the same trigger ourselves and *bound* the
    // drain: the SSE and long-poll endpoints hold a connection open indefinitely,
    // so an unbounded graceful wait would never return.
    // `into_make_service_with_connect_info` exposes the peer address so the
    // account-link callback can enforce loopback-only (see server::account).
    //
    // Each listener serves the *same* router with one extra layer naming the
    // acceptor. That layer is added outermost, so it is in place before the gate
    // reads it, and it is the only thing that distinguishes the two — one router,
    // one set of handlers, one behaviour, differing solely in who is allowed to
    // reach them.
    use foundation::surfaces::{Acceptor, accepted_on};
    let server_shutdown = shutdown.clone();
    let loopback_router = accepted_on(router.clone(), Acceptor::Loopback);
    let mut server = tokio::spawn(async move {
        axum::serve(
            listener,
            loopback_router.into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .with_graceful_shutdown(shutdown_requested(server_shutdown))
        .await
    });

    // The off-box server drains on the same trigger. It is a separate task rather
    // than a second `select!` arm because it may not exist at all, and the
    // loopback one is the one whose exit means "we are done serving".
    let off_box_server = off_box_listener.map(|l| {
        let router = accepted_on(router, Acceptor::OffBox);
        let shutdown = shutdown.clone();
        tokio::spawn(async move {
            let r = axum::serve(
                l,
                router.into_make_service_with_connect_info::<std::net::SocketAddr>(),
            )
            .with_graceful_shutdown(shutdown_requested(shutdown))
            .await;
            if let Err(e) = r {
                tracing::error!(error = %e, "off-box HTTP server error");
            }
        })
    });
    let _ = server_ready_tx.send(true);

    tokio::select! {
        joined = &mut server => match joined {
            Ok(Ok(())) => tracing::info!("HTTP server stopped"),
            Ok(Err(e)) => tracing::error!(error = %e, "HTTP server error"),
            Err(e) => tracing::error!(error = %e, "HTTP server task panicked"),
        },
        _ = shutdown_requested(shutdown.clone()) => {
            // Quiesce the reaction *first*, before the drain: for the whole drain
            // window it must not restart a session or open a reflection pass, or a
            // freshly spawned child would race the reap below and outlive us.
            reaction_shutdown.trigger();
            tracing::info!(grace = ?SHUTDOWN_GRACE, "shutdown requested; draining in-flight requests");
            match tokio::time::timeout(SHUTDOWN_GRACE, &mut server).await {
                Ok(Ok(Ok(()))) => tracing::info!("HTTP server drained cleanly"),
                Ok(Ok(Err(e))) => tracing::error!(error = %e, "HTTP server error during drain"),
                Ok(Err(e)) => tracing::error!(error = %e, "HTTP server task panicked during drain"),
                Err(_) => {
                    tracing::warn!(grace = ?SHUTDOWN_GRACE, "drain grace elapsed; aborting in-flight connections");
                    server.abort();
                }
            }
        }
    }

    // Also quiesce the reaction if we got here via the server task ending on its own
    // (an HTTP error, not a signal) — idempotent, and it stops the reaction from
    // respawning sessions while we reap. No-op on the shutdown path (already fired).
    reaction_shutdown.trigger();

    // Let the off-box acceptor finish its own drain, bounded by the same grace. It
    // holds the identical long-lived connections (SSE, the long-polls), so an
    // unbounded wait here would hang exit exactly as it would there.
    if let Some(mut task) = off_box_server {
        if tokio::time::timeout(SHUTDOWN_GRACE, &mut task).await.is_err() {
            tracing::warn!("off-box drain grace elapsed; aborting its connections");
            task.abort();
        }
    }
    if let Some(mut task) = app_server {
        if tokio::time::timeout(SHUTDOWN_GRACE, &mut task).await.is_err() {
            tracing::warn!("the app's drain grace elapsed; aborting its connections");
            task.abort();
        }
    }

    // Reap every codex subprocess (one per live session) so none
    // are orphaned. Bounded so a stuck child can't hang exit.
    if tokio::time::timeout(SHUTDOWN_GRACE, agent_for_shutdown.shutdown())
        .await
        .is_err()
    {
        tracing::warn!("codex subprocess reaping timed out");
    }

    // Close whatever is still on the switchboard, so the session directory records that
    // *we* ended these rather than leaving them to be read as lost.
    //
    // Not every rung unregisters on its own here: Reflection holds a scope-bound
    // `Registration` whose `Drop` runs when its task winds down, while Reaction and
    // Cognition are registered for the life of the process and simply stop existing with
    // it. Without this, every clean stop left them with an
    // `opened` and no `closed` — which is the exact signature of a crash, so the roster
    // would report "lost to a restart" after an ordinary quit and the warning would stop
    // meaning anything.
    foundation::registry::global().close_all();
    // And wait for them to land: the records are queued to a writer task that the runtime
    // is about to drop. Bounded, because a stuck disk must not hang exit — a lost record
    // reads as a crash, which is a worse outcome than a slow quit but a better one than
    // never quitting.
    if tokio::time::timeout(SHUTDOWN_GRACE, foundation::registry::global().flush_index())
        .await
        .is_err()
    {
        tracing::warn!("session directory flush timed out; the last closes may read as lost");
    }

    tracing::info!("hi-agent shut down");
    Ok(())
}

/// macOS entry: run the menu-bar status item on the **main thread** (AppKit's
/// `NSStatusItem` requires it) while the HTTP server + reaction run on a background
/// thread with their own runtime. This is the inversion the one-binary
/// distribution model accepted as the cost of a tray: elsewhere tokio owns the
/// main thread, here AppKit does.
///
/// The config is built on the server thread (via `build_config`) rather than before
/// this call, so a missing/invalid key keeps the app alive and visible instead of
/// aborting before the menu-bar item appears. On any startup failure the server
/// thread marks the icon, reveals `<data_dir>/.env` for editing, and waits for Quit
/// — the app stays up so the user can read the problem and fix it.
///
/// The tray's "Quit" notifies `shutdown`; the server thread observes it, runs the
/// normal graceful drain + subprocess reap, then exits the process — which also tears
/// down the main-thread AppKit loop. If the status item can't be created (e.g. no
/// window-server session), the agent falls back to running headless rather than
/// failing.
#[cfg(target_os = "macos")]
pub fn run_with_tray(
    port: u16,
    app_port: Option<u16>,
    data_dir: PathBuf,
    build_config: impl FnOnce() -> anyhow::Result<Config> + Send + 'static,
) -> anyhow::Result<()> {
    // "Open" opens the **app**, which is the thing with a face; the core's own
    // port is an internal address the app forwards to. They are the same process
    // today and will not always be.
    let url = format!("http://127.0.0.1:{}/", app_port.unwrap_or(port));
    let shutdown = Arc::new(Notify::new());

    // The tray's Account submenu reads/writes the credential store, so it needs the
    // data dir too; clone it before the server thread's closure moves the original.
    let tray_data_dir = data_dir.clone();

    let server_shutdown = shutdown.clone();
    let server = std::thread::Builder::new()
        .name("hi-agent-server".to_string())
        .spawn(move || {
            let rt = match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
                Ok(rt) => rt,
                Err(e) => {
                    tracing::error!(error = %e, "failed to build server runtime");
                    std::process::exit(1);
                }
            };
            // Build the config here, on the server thread, so a bad/missing config
            // doesn't abort before the menu-bar item is up. On failure keep the app
            // alive: mark the icon, put the `.env` to edit in front of the user, and
            // wait for Quit — rather than vanishing with no trace.
            let config = match build_config() {
                Ok(config) => config,
                Err(e) => {
                    tracing::error!(error = %format!("{e:#}"), "configuration error — fix it and relaunch; the menu-bar app stays up");
                    body::capabilities::tray::set_text("⚠ needs setup");
                    // Reveal the file to edit (best-effort; needs a window-server
                    // session, so a no-op when headless).
                    let _ = std::process::Command::new("open")
                        .arg("-R")
                        .arg(data_dir.join(".env"))
                        .spawn();
                    rt.block_on(server_shutdown.notified());
                    std::process::exit(0);
                }
            };
            match rt.block_on(run_with_shutdown(config, server_shutdown.clone())) {
                // Graceful shutdown completed (drained + codex subprocesses reaped).
                // Exit the process, which also stops the main-thread AppKit loop.
                Ok(()) => std::process::exit(0),
                Err(e) => {
                    tracing::error!(error = %format!("{e:#}"), "hi-agent server exited with error; the menu-bar app stays up");
                    body::capabilities::tray::set_text("⚠ startup failed");
                    rt.block_on(server_shutdown.notified());
                    std::process::exit(0);
                }
            }
        })
        .context("spawning server thread")?;

    // Blocks on the AppKit run loop until the process exits via the server thread
    // above. Returns early only if the status item can't be created — in which
    // case fall back to running headless by joining the server.
    if let Err(e) = body::capabilities::tray::run(url, tray_data_dir, shutdown) {
        tracing::warn!(error = %format!("{e:#}"), "menu-bar item unavailable; running without it");
    }
    let _ = server.join();
    Ok(())
}

/// How long in-flight HTTP requests get to finish after a shutdown signal — and,
/// separately, the budget for reaping codex subprocesses — before we stop waiting
/// and exit anyway.
const SHUTDOWN_GRACE: Duration = Duration::from_secs(10);

/// Resolves when shutdown is requested by an OS signal (SIGINT/SIGTERM) **or** by
/// the `extra` trigger (the tray's Quit). Takes the `Notify` by `Arc` so it can be
/// moved into the server task's graceful-shutdown future. See [`shutdown_signal`]
/// for the signal half.
async fn shutdown_requested(extra: Arc<Notify>) {
    tokio::select! {
        _ = shutdown_signal() => {}
        _ = extra.notified() => {}
    }
}

/// Resolves on the first SIGINT (Ctrl-C) or SIGTERM. Each call registers fresh
/// listeners, and tokio delivers the signal to all of them, so it is safe to
/// await in more than one place (the server's graceful-shutdown future and the
/// drain supervisor both use it). A failure to install a handler logs and then
/// parks forever, so it never spuriously triggers shutdown.
async fn shutdown_signal() {
    let ctrl_c = async {
        if let Err(e) = tokio::signal::ctrl_c().await {
            tracing::error!(error = %e, "failed to listen for ctrl-c");
            std::future::pending::<()>().await;
        }
    };

    #[cfg(unix)]
    let terminate = async {
        use tokio::signal::unix::{SignalKind, signal};
        match signal(SignalKind::terminate()) {
            Ok(mut stream) => {
                stream.recv().await;
            }
            Err(e) => {
                tracing::error!(error = %e, "failed to install SIGTERM handler");
                std::future::pending::<()>().await;
            }
        }
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {}
        _ = terminate => {}
    }
}
