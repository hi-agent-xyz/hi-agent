//! Broker client — bootstrap a xiaoyuanzhu account and fetch configs + energy from
//! the broker (hi-agent.xyz).
//!
//! Xiaoyuanzhu mode: the `device_id` seeds a one-time **bootstrap** that
//! auto-creates the account at the broker and returns OAuth tokens; thereafter the
//! access token authenticates `/configs` (rare) and `/energy` (frequent), refreshed
//! via the refresh token. After bootstrap the broker only ever sees one identity —
//! the account token — so the anonymous (`free` tier) and signed-in (`sub` tier)
//! accounts share one path. Signed-in bootstrap (Authentik-authenticated, seeded by
//! `bearer`) is future work; today an anonymous device account is always minted.

use std::path::Path;
use std::sync::OnceLock;
use std::time::Duration;

use anyhow::Context;
use serde::Deserialize;

use crate::foundation::credentials::{
    Credentials, Energy, Identity, LlmCredentials, Managed, ModelOffer, Mode, Tokens, VendorKey,
};

/// Env override for the broker base URL (default [`DEFAULT_BROKER_URL`]).
const ENV_BROKER_URL: &str = "HI_AGENT_BROKER_URL";
const DEFAULT_BROKER_URL: &str = "https://hi-agent.xyz";
/// Public account site. Kept separate from the broker API origin so account links
/// can move without redirecting credential and energy traffic.
const ENV_PUBLIC_URL: &str = "HI_AGENT_PUBLIC_URL";
const DEFAULT_PUBLIC_URL: &str = "https://hi-agent.xyz";
/// Renew before the bearer is close enough to expiry that a tunnel dial or
/// registry request could lose the race against it.
const ACCOUNT_TOKEN_MIN_VALIDITY: chrono::Duration = chrono::Duration::minutes(5);

/// Refresh tokens rotate. Every path that can exchange or replace one must be
/// serialized so two concurrent callers cannot persist different generations.
static TOKEN_REFRESH: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();

fn token_refresh_lock() -> &'static tokio::sync::Mutex<()> {
    TOKEN_REFRESH.get_or_init(|| tokio::sync::Mutex::new(()))
}

/// `app_settings` keys recording the outcome of the last broker sync, so the
/// Settings page can show a real state (connecting / connected / problem) instead
/// of a perpetual "connecting". Written on every refresh + energy poll; read by
/// the public `/api/account` status endpoint. Not secrets.
pub const KEY_BROKER_STATE: &str = "broker_state"; // "ok" | "error"
pub const KEY_BROKER_ERROR: &str = "broker_error"; // last error text (cleared on ok)
pub const KEY_BROKER_CHECKED_AT: &str = "broker_checked_at"; // rfc3339 of last attempt

fn base_url() -> String {
    std::env::var(ENV_BROKER_URL)
        .ok()
        .map(|s| s.trim().trim_end_matches('/').to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| DEFAULT_BROKER_URL.to_string())
}

/// The public site URL used for account and subscription links.
pub fn public_base_url() -> String {
    std::env::var(ENV_PUBLIC_URL)
        .ok()
        .map(|s| s.trim().trim_end_matches('/').to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| DEFAULT_PUBLIC_URL.to_string())
}

/// Bounded HTTP client so a slow/unreachable broker can't hang the boot path.
fn http() -> anyhow::Result<reqwest::Client> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(8))
        // Never follow a redirect on the broker API. A 3xx here is always a
        // misconfiguration — a moved origin, a stale constant — and following it
        // makes that misconfiguration *quiet* in the one case that matters. On a
        // 301, reqwest downgrades POST to GET and drops the body (browser
        // semantics), so a `POST /api/agent/bootstrap` arrives as a GET, misses
        // the POST-only route, falls through to the site's SPA handler, and comes
        // back `200 text/html`. That sails past the status check and dies at
        // `resp.json()` as "parsing bootstrap response" — an error naming nothing
        // that is wrong. The GET endpoints (`/configs`, `/energy`) meanwhile
        // follow the redirect and keep working, so the origin looks healthy while
        // every POST is silently destroyed. Refusing the redirect surfaces the
        // 301 itself, whose `Location` says exactly where the broker went.
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .context("building broker http client")
}

/// Coarse, non-identifying device telemetry sent on bootstrap (sanity-check only).
fn device_info() -> serde_json::Value {
    serde_json::json!({
        "os": std::env::consts::OS,
        "arch": std::env::consts::ARCH,
        "app_version": env!("CARGO_PKG_VERSION"),
        "install_shape": std::env::var("HI_AGENT_INSTALL_SHAPE")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| "unknown".to_string()),
    })
}

#[derive(Deserialize, Default)]
struct TokenDto {
    #[serde(default)]
    access_token: String,
    #[serde(default)]
    refresh_token: String,
    #[serde(default)]
    expires_in: i64,
}

/// One model the broker offers for a task, with editorial 0–100 scores. Today we
/// select purely on `quality`; `speed`/`price` are parsed so the shape round-trips
/// and a smarter policy can weigh them later.
#[derive(Deserialize, Default, Clone)]
struct ModelDto {
    #[serde(default)]
    model: String,
    /// Optional cheaper/faster companion for the background "haiku" slot. Empty →
    /// the client reuses `model` for that slot. Only meaningful on the LLM task.
    #[serde(default)]
    small: String,
    #[serde(default)]
    quality: i64,
    // Parsed for round-trip + a future weighted policy; selection uses quality today.
    #[serde(default)]
    #[allow(dead_code)]
    speed: i64,
    #[serde(default)]
    #[allow(dead_code)]
    price: i64,
}

/// One wire's endpoint: the full songguo URL for that protocol, the shared token,
/// and the models served over it.
#[derive(Deserialize, Default, Clone)]
struct WireDto {
    #[serde(default)]
    url: String,
    #[serde(default)]
    api_key: String,
    #[serde(default)]
    models: Vec<ModelDto>,
    /// The mainline model that carries a hosted tool, on a wire whose work happens
    /// inside a turn rather than at its own endpoint (`openai-responses` serving
    /// `text-to-image`). Absent on every other wire, and the capability that needs one
    /// refuses to start without it rather than inventing a model to bill.
    #[serde(default)]
    carrier: String,
}

/// GET /api/agent/configs — a three-layer menu: HF task name → wire name →
/// endpoint. Collapsed into the internal per-slot [`Managed`] by [`managed_from`].
type ConfigsDto = std::collections::HashMap<String, std::collections::HashMap<String, WireDto>>;

/// The broker's name for the wire the agent speaks. Codex drives the OpenAI
/// **Responses** API, and the broker already mints this wire alongside the others, so
/// the swap away from Claude Code needed no broker change — only that the client stop
/// asking for `anthropic-messages`.
const BROKER_LLM_WIRE: &str = "openai-responses";

/// Reduce the broker's full endpoint (`…/v1/responses`) to the provider *base* codex
/// wants (`…/v1`), which its `wire_api = "responses"` appends `/responses` to. Only the
/// endpoint leaf is stripped — the `/v1` prefix is part of the base.
fn openai_responses_base(url: &str) -> String {
    let u = url.trim().trim_end_matches('/');
    u.strip_suffix("/responses").unwrap_or(u).to_string()
}

/// The LLM wire from the broker's menu, or `None` when it offers none we can drive.
///
/// **The one task that is deliberately single-wire.** Every other task hands the whole
/// menu to its capability to choose from ([`managed_from`]); here the wire is not ours
/// to choose, because the wire *is* the agent runtime — codex speaks OpenAI Responses,
/// so `anthropic-messages` (which the broker still lists for older installs) would be a
/// second engine rather than a second config, and we ignore it outright.
///
/// Takes the highest-`quality` model on that wire, plus its optional `small` companion
/// for the background slot.
fn pick_llm_wire(c: &ConfigsDto) -> Option<(String, String, Option<String>, Option<String>)> {
    let w = c.get("text-generation")?.get(BROKER_LLM_WIRE)?;
    let best = w.models.iter().max_by_key(|m| m.quality);
    let model = best.map(|m| m.model.trim().to_string()).filter(|s| !s.is_empty());
    let small = best.map(|m| m.small.trim().to_string()).filter(|s| !s.is_empty());
    Some((openai_responses_base(w.url.trim()), w.api_key.clone(), model, small))
}

/// Collapse the broker menu into the internal per-slot [`Managed`], selecting the
/// best-quality model per task.
///
/// Our code treats the broker's **full endpoint URL** as the source of truth. Most
/// capabilities use it verbatim; the LLM wire strips only the endpoint leaf, keeping
/// the OpenAI `/v1` base that codex's provider block wants.
fn managed_from(c: &ConfigsDto) -> Managed {
    // **Every wire the broker offers for the task, best first — none discarded here.**
    //
    // Which HTTP shapes we can actually speak is knowledge that lives in the
    // capability, not in this function, so choosing among the wires is the
    // capability's job; the broker client's job is to hand over the whole menu.
    // Collapsing it here is what hid `gpt-image-2`: songguo served it over
    // `openai-responses` while `openai-images` served seedream, and the survivor was
    // whichever wire sorted first *by name*.
    //
    // Each wire keeps its id **and its whole model list**, not just the best-quality
    // pick — for the generation capabilities that list *is* the menu the agent chooses
    // from, and collapsing it is what made "the agent picks the model" unimplementable.
    // Every capability now reads the wire id loosely and skips what it cannot speak, so
    // an unfamiliar spelling (`volc-asr-stream-async`) costs a log line rather than the
    // boot it cost when this passed the id through the first time.
    let wires_for = |task: &str| -> Vec<VendorKey> {
        let Some(wires) = c.get(task) else { return Vec::new() };
        let mut slots: Vec<VendorKey> = wires
            .iter()
            .map(|(wire, w)| {
                let best = w.models.iter().max_by_key(|m| m.quality);
                VendorKey {
                    wire: wire.clone(),
                    base_url: w.url.trim().to_string(),
                    api_key: w.api_key.clone(),
                    model: best.map(|m| m.model.trim().to_string()).filter(|s| !s.is_empty()),
                    carrier: w.carrier.trim().to_string(),
                    models: w
                        .models
                        .iter()
                        .filter(|m| !m.model.trim().is_empty())
                        .map(|m| ModelOffer {
                            name: m.model.trim().to_string(),
                            quality: m.quality,
                            speed: m.speed,
                            price: m.price,
                        })
                        .collect(),
                }
            })
            .collect();
        // Rank by the best model each wire serves, so "first" means "best on offer"
        // rather than "alphabetically luckiest". The wire name breaks ties only to keep
        // the order stable across runs — a `HashMap` iterates differently every time,
        // and a capability that takes the first would otherwise pick a different vendor
        // on each boot.
        slots.sort_by(|a, b| {
            let q = |v: &VendorKey| v.models.iter().map(|m| m.quality).max().unwrap_or(i64::MIN);
            q(b).cmp(&q(a)).then_with(|| a.wire.cmp(&b.wire))
        });
        slots
    };
    let llm = pick_llm_wire(c)
        .map(|(base_url, api_key, model, small)| LlmCredentials {
            wire: BROKER_LLM_WIRE.to_string(),
            base_url,
            api_key,
            model,
            small,
        })
        .unwrap_or_default();
    Managed {
        llm,
        stt: wires_for("automatic-speech-recognition"),
        tts: wires_for("text-to-speech"),
        vision: wires_for("image-text-to-text"),
        image: wires_for("text-to-image"),
        video: wires_for("text-to-video"),
    }
}

#[derive(Deserialize, Default)]
struct EnergyDto {
    #[serde(default)]
    remaining: i64,
    #[serde(default)]
    total: i64,
    #[serde(default)]
    resets_at: String,
    #[serde(default)]
    tier: String,
}

fn tokens_from(t: TokenDto) -> Tokens {
    let expires_at = (chrono::Utc::now() + chrono::Duration::seconds(t.expires_in.max(0))).to_rfc3339();
    Tokens {
        access_token: t.access_token,
        refresh_token: t.refresh_token,
        access_expires_at: expires_at,
    }
}

fn fresh_access_token(tokens: &Tokens, min_validity: chrono::Duration) -> Option<String> {
    let access = tokens.access_token.trim();
    if access.is_empty() {
        return None;
    }
    let expires = chrono::DateTime::parse_from_rfc3339(tokens.access_expires_at.trim()).ok()?;
    (expires > chrono::Utc::now() + min_validity).then(|| access.to_string())
}

/// POST /api/agent/bootstrap — free device → account tokens (auto-creates the
/// account at the broker on first contact).
async fn bootstrap(device_id: &str) -> anyhow::Result<Tokens> {
    let url = format!("{}/api/agent/bootstrap", base_url());
    let resp = http()?
        .post(&url)
        .json(&serde_json::json!({
            "mode": "free",
            "device_id": device_id,
            "device_info": device_info(),
        }))
        .send()
        .await
        .with_context(|| format!("calling {url}"))?;
    let status = resp.status();
    if !status.is_success() {
        anyhow::bail!("bootstrap {url} returned {status}: {}", resp.text().await.unwrap_or_default());
    }
    Ok(tokens_from(resp.json().await.context("parsing bootstrap response")?))
}

/// POST /api/agent/token — refresh_token grant (the broker rotates the refresh
/// token, so the returned pair must be persisted).
async fn refresh_access(refresh_token: &str) -> anyhow::Result<Tokens> {
    let url = format!("{}/api/agent/token", base_url());
    let resp = http()?
        .post(&url)
        .json(&serde_json::json!({ "grant_type": "refresh_token", "refresh_token": refresh_token }))
        .send()
        .await
        .with_context(|| format!("calling {url}"))?;
    let status = resp.status();
    if !status.is_success() {
        anyhow::bail!("token {url} returned {status}: {}", resp.text().await.unwrap_or_default());
    }
    Ok(tokens_from(resp.json().await.context("parsing token response")?))
}

/// GET /api/agent/configs — the vendor settings to apply (rare fetch).
async fn fetch_configs(access: &str) -> anyhow::Result<Managed> {
    let url = format!("{}/api/agent/configs", base_url());
    let resp = http()?
        .get(&url)
        .bearer_auth(access)
        .send()
        .await
        .with_context(|| format!("calling {url}"))?;
    let status = resp.status();
    if !status.is_success() {
        anyhow::bail!("configs {url} returned {status}: {}", resp.text().await.unwrap_or_default());
    }
    let c: ConfigsDto = resp.json().await.context("parsing configs")?;
    Ok(managed_from(&c))
}

/// GET /api/agent/energy — the user-facing balance (frequent poll).
pub async fn fetch_energy(access: &str) -> anyhow::Result<Energy> {
    let url = format!("{}/api/agent/energy", base_url());
    let resp = http()?
        .get(&url)
        .bearer_auth(access)
        .send()
        .await
        .with_context(|| format!("calling {url}"))?;
    let status = resp.status();
    if !status.is_success() {
        anyhow::bail!("energy {url} returned {status}: {}", resp.text().await.unwrap_or_default());
    }
    let e: EnergyDto = resp.json().await.context("parsing energy")?;
    Ok(Energy { remaining: e.remaining, total: e.total, resets_at: e.resets_at, tier: e.tier })
}

#[derive(Deserialize, Default)]
struct WebTicketDto {
    #[serde(default)]
    ticket: String,
    #[serde(default)]
    path: String,
}

/// Mint a one-time web-handoff ticket and return the browser URL that lands the
/// user on the site **already signed in as this device account**
/// (`<site><path>?ticket=…`). The tray's "Subscribe" and the out-of-energy view
/// pass `prefer_path = Some("/account")`. It uses the same renewable account
/// token as the registry and relay, independent of model-provider mode. The
/// ticket is a URL-safe JWT (base64url + dots), so no query-encoding is needed.
pub async fn subscribe_url(data_dir: &Path, prefer_path: Option<&str>) -> anyhow::Result<String> {
    let access = account_token(data_dir)
        .await
        .context("ensuring an account token before web handoff")?;
    let url = format!("{}/api/agent/web-ticket", base_url());
    let resp = http()?
        .post(&url)
        .bearer_auth(&access)
        .send()
        .await
        .with_context(|| format!("calling {url}"))?;
    let status = resp.status();
    if !status.is_success() {
        anyhow::bail!("web-ticket {url} returned {status}: {}", resp.text().await.unwrap_or_default());
    }
    let dto: WebTicketDto = resp.json().await.context("parsing web-ticket response")?;
    if dto.ticket.trim().is_empty() {
        anyhow::bail!("broker returned an empty web ticket");
    }
    // The caller's preferred landing page wins (the energy view wants `/account`); else the
    // broker's suggested path, else the account page. The ticket is a login handoff,
    // valid for any page on the domain, so overriding the path is safe.
    let prefer = prefer_path.map(|p| p.trim()).filter(|p| !p.is_empty());
    let broker_path = dto.path.trim();
    let path = prefer.unwrap_or(if broker_path.is_empty() { "/account" } else { broker_path });
    Ok(format!(
        "{}{}?ticket={}",
        public_base_url(),
        path,
        dto.ticket.trim()
    ))
}

/// Exchange or recover the account token pair. The caller must hold
/// [`token_refresh_lock`] because the broker rotates refresh tokens. On a failed
/// refresh, re-bootstrap on the stable `device_id`, which resolves to the same
/// device account.
async fn ensure_tokens(store: &Credentials) -> anyhow::Result<Tokens> {
    if let Some(t) = &store.tokens {
        if !t.refresh_token.trim().is_empty() {
            match refresh_access(&t.refresh_token).await {
                Ok(nt) => return Ok(nt),
                // Expected, self-healing path: the broker rotates/expires refresh
                // tokens, and a stale one (prior run, broker DB reset) is idempotently
                // recovered by re-bootstrapping on the stable device_id. Because the
                // broker keys accounts on device_id, this re-resolves the SAME account
                // — no new account is created. Not fatal.
                Err(e) => tracing::warn!(error = %format!("{e:#}"), "broker refresh failed; re-resolving device account (idempotent on device_id, same account)"),
            }
        }
    }
    anyhow::ensure!(
        !store.device_id.trim().is_empty(),
        "cannot recover the account token without a stable device id"
    );
    bootstrap(&store.device_id).await
}

/// Return an account bearer suitable for registry and relay traffic.
///
/// This lifecycle is independent of provider mode: switching to BYOK changes
/// model credentials, not the account that owns a handle. A usable cached token
/// is returned directly; an expired one is refreshed and persisted. Re-bootstrap
/// is only attempted when this install already has a stable device relationship,
/// so a fresh unregistered install still gets the actionable "sign in" error.
pub async fn account_token(data_dir: &Path) -> anyhow::Result<String> {
    let _guard = token_refresh_lock().lock().await;
    let mut store = Credentials::load(data_dir);

    if let Some(access) = store
        .tokens
        .as_ref()
        .and_then(|tokens| fresh_access_token(tokens, ACCOUNT_TOKEN_MIN_VALIDITY))
    {
        return Ok(access);
    }

    let has_tokens = store.tokens.as_ref().is_some_and(|tokens| {
        !tokens.access_token.trim().is_empty() || !tokens.refresh_token.trim().is_empty()
    });
    anyhow::ensure!(
        has_tokens || !store.device_id.trim().is_empty(),
        "no account yet — sign in, then claim a name"
    );

    let tokens = ensure_tokens(&store)
        .await
        .context("renewing the account token")?;
    let access = tokens.access_token.trim().to_string();
    anyhow::ensure!(!access.is_empty(), "broker returned an empty account token");
    store.tokens = Some(tokens);
    store
        .save(data_dir)
        .context("saving the renewed account token")?;
    Ok(access)
}

/// The signed-in account's identity in a claim response.
#[derive(Deserialize, Default)]
struct ClaimIdentityDto {
    #[serde(default)]
    email: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    tier: String,
}

/// POST /api/agent/claim success body: fresh account tokens + the adopted identity.
#[derive(Deserialize, Default)]
struct ClaimDto {
    #[serde(default)]
    access_token: String,
    #[serde(default)]
    refresh_token: String,
    #[serde(default)]
    expires_in: i64,
    #[serde(default)]
    identity: ClaimIdentityDto,
}

/// A broker error body (`{error, message}`) — read to surface the 409 conflict code.
#[derive(Deserialize, Default)]
struct ErrorDto {
    #[serde(default)]
    error: String,
}

/// The result of a web→device claim.
pub enum ClaimOutcome {
    /// The device adopted the signed-in account; `email` is who it now is.
    Adopted { email: String },
    /// The broker declined to switch automatically — `code` is the machine reason
    /// (`keep_current` = a recoverable account is signed in here; `chooser_required`
    /// = both accounts are bound). The caller explains it to the user.
    Conflict { code: String },
}

/// Redeem a device-ticket to adopt the account the browser is signed in as (the
/// web→device link). Authenticates with this device's current access token so the
/// broker can apply its adoption policy (A = this device, B = the ticket's account);
/// on success it relinks the device server-side and returns fresh tokens for B,
/// which we swap into the store along with B's identity, then refresh configs +
/// energy under the new tokens. A 409 comes back as [`ClaimOutcome::Conflict`] for
/// the caller to explain rather than an error. Xiaoyuanzhu mode only.
pub async fn claim_device(data_dir: &Path, ticket: &str) -> anyhow::Result<ClaimOutcome> {
    let email = {
        let _guard = token_refresh_lock().lock().await;
        let mut store = Credentials::load(data_dir);
        if store.mode == Mode::Byok {
            anyhow::bail!("account linking is only available in xiaoyuanzhu mode");
        }
        if store.device_id.trim().is_empty() {
            store.device_id = crate::foundation::machine_id::derive()
                .unwrap_or_else(|| uuid::Uuid::now_v7().to_string());
        }

        // The claim is authed as the device's current account. Keep this whole
        // adoption under the refresh lock: success replaces the account token
        // pair, so an hourly refresh must not overwrite it with the old account.
        let tokens = if let Some(access) = store
            .tokens
            .as_ref()
            .and_then(|tokens| fresh_access_token(tokens, ACCOUNT_TOKEN_MIN_VALIDITY))
        {
            let mut tokens = store.tokens.clone().unwrap_or_default();
            tokens.access_token = access;
            tokens
        } else {
            ensure_tokens(&store)
                .await
                .context("ensuring a device token before claim")?
        };
        store.tokens = Some(tokens.clone());
        store
            .save(data_dir)
            .context("saving the device token before claim")?;

        let url = format!("{}/api/agent/claim", base_url());
        let resp = http()?
            .post(&url)
            .bearer_auth(&tokens.access_token)
            .json(&serde_json::json!({ "ticket": ticket }))
            .send()
            .await
            .with_context(|| format!("calling {url}"))?;
        let status = resp.status();
        if status.as_u16() == 409 {
            let code = resp
                .json::<ErrorDto>()
                .await
                .map(|e| e.error)
                .unwrap_or_default();
            return Ok(ClaimOutcome::Conflict { code });
        }
        if !status.is_success() {
            anyhow::bail!(
                "claim {url} returned {status}: {}",
                resp.text().await.unwrap_or_default()
            );
        }
        let dto: ClaimDto = resp.json().await.context("parsing claim response")?;

        // Swap in the adopted account's tokens + identity.
        store.tokens = Some(tokens_from(TokenDto {
            access_token: dto.access_token,
            refresh_token: dto.refresh_token,
            expires_in: dto.expires_in,
        }));
        let email = dto.identity.email.clone();
        store.identity = if email.trim().is_empty() {
            None
        } else {
            Some(Identity {
                email: email.clone(),
                name: dto.identity.name,
                tier: dto.identity.tier,
            })
        };
        store.save(data_dir).context("saving claimed account")?;
        email
    };

    // Pull the adopted account's configs + energy under its new tokens. `refresh`
    // reloads the store (keeping the identity we just wrote) and persists again.
    refresh(data_dir, None).await;
    Ok(ClaimOutcome::Adopted { email })
}

/// Persist the last broker-sync outcome to `app_settings` (best-effort — a failed
/// write just leaves the UI showing a slightly stale state). On success the stored
/// error is cleared; on failure its text is kept so the Settings page can surface
/// *why* the account is unavailable rather than spinning on "connecting".
fn record_status(data_dir: &Path, ok: bool, error: &str) {
    use crate::foundation::credentials::set_setting;
    let now = chrono::Utc::now().to_rfc3339();
    let _ = set_setting(data_dir, KEY_BROKER_STATE, if ok { "ok" } else { "error" });
    let _ = set_setting(data_dir, KEY_BROKER_ERROR, if ok { "" } else { error });
    let _ = set_setting(data_dir, KEY_BROKER_CHECKED_AT, &now);
}

/// In xiaoyuanzhu mode: ensure account tokens (bootstrap/refresh), fetch configs +
/// energy, and persist. Best-effort — failures log and keep any cached configs.
/// Derives (or, as a fallback, mints) a `device_id` on first need — see
/// `foundation::machine_id`. No-op in BYOK. The `bearer` (a signed-in
/// Authentik session, when present) will seed the `sub`-tier bootstrap once that's
/// wired; today an anonymous device account is always minted. v1 runs at startup
/// and on mode-select; a periodic loop is wired in `lib.rs`.
pub async fn refresh(data_dir: &Path, bearer: Option<&str>) {
    let _ = bearer; // reserved for signed-in (`sub`-tier) bootstrap; unused today.

    let tokens = {
        let _guard = token_refresh_lock().lock().await;
        let mut store = Credentials::load(data_dir);
        match store.mode {
            Mode::Byok => return,
            Mode::Xiaoyuanzhu => {}
        }

        if store.device_id.trim().is_empty() {
            // First need: prefer a machine-derived id so the account survives an app
            // uninstall / data-dir wipe (the broker keys one account per device_id).
            // Fall back to a random UUID only when no platform source is readable. An
            // install that already holds a device_id keeps it — we never re-derive out
            // from under a live account. See `foundation::machine_id`.
            store.device_id = crate::foundation::machine_id::derive()
                .unwrap_or_else(|| uuid::Uuid::now_v7().to_string());
        }

        let tokens = match ensure_tokens(&store).await {
            Ok(t) => t,
            Err(e) => {
                tracing::warn!(error = %format!("{e:#}"), "broker bootstrap/refresh failed; keeping cached configs");
                record_status(data_dir, false, &format!("{e:#}"));
                if let Err(e) = store.save(data_dir) {
                    tracing::warn!(error = %format!("{e:#}"), "failed to persist credential store");
                }
                return;
            }
        };
        store.tokens = Some(tokens.clone());
        if let Err(e) = store.save(data_dir) {
            tracing::warn!(error = %format!("{e:#}"), "failed to persist renewed account tokens");
        }
        tokens
    };

    let mut managed = None;
    let mut energy = None;
    match fetch_configs(&tokens.access_token).await {
        Ok(m) => {
            tracing::info!("fetched managed configs from broker");
            managed = Some(m);
        }
        Err(e) => tracing::warn!(error = %format!("{e:#}"), "configs fetch failed; keeping cached"),
    }
    match fetch_energy(&tokens.access_token).await {
        Ok(en) => {
            tracing::info!(tier = %en.tier, remaining = en.remaining, total = en.total, "energy refreshed");
            energy = Some(en);
        }
        Err(e) => tracing::warn!(error = %format!("{e:#}"), "energy fetch failed; keeping cached"),
    }

    let applied_energy = {
        // Reload under the token lock after the network calls so an account
        // adoption or settings write that completed meanwhile is preserved.
        // If the account token changed, these responses belong to the previous
        // account and must be discarded.
        let _guard = token_refresh_lock().lock().await;
        let mut store = Credentials::load(data_dir);
        let current_access = store
            .tokens
            .as_ref()
            .map(|t| t.access_token.trim())
            .unwrap_or_default();
        if current_access == tokens.access_token.trim() {
            if let Some(m) = managed {
                store.managed = Some(m);
            }
            if let Some(en) = energy.as_ref() {
                store.energy = Some(en.clone());
            }
            match store.save(data_dir) {
                Ok(()) => energy,
                Err(e) => {
                    tracing::warn!(error = %format!("{e:#}"), "failed to persist credential store after broker refresh");
                    None
                }
            }
        } else {
            tracing::debug!("discarding broker data fetched before the account changed");
            None
        }
    };
    if let Some(en) = applied_energy {
        // Balance is recovery-only. Calls run until a managed provider actually
        // returns 402; a later positive balance emits Resume.
        crate::foundation::energy_state::reconcile(data_dir, en.remaining, en.total);
        crate::foundation::energy_history::record(data_dir, &en);
    }

    // Tokens were obtained → the account exists and is healthy, even if a vendor
    // sub-fetch above degraded. Record success so the UI leaves "connecting".
    record_status(data_dir, true, "");
}

/// Lightweight energy poll that hands back the fresh balance: re-fetch with the
/// cached access token, persist it, and return it. `None` in BYOK, when no token
/// is cached yet, or when the fetch fails (the last cached value is left in
/// place). The account endpoint uses this to refresh the ground-truth balance; the
/// reconciliation inside this function emits Resume when energy returns.
pub async fn poll_energy_now(data_dir: &Path) -> Option<Energy> {
    if Credentials::load(data_dir).mode == Mode::Byok {
        return None;
    }
    let access = match account_token(data_dir).await {
        Ok(access) => access,
        Err(e) => {
            tracing::debug!(error = %format!("{e:#}"), "energy poll could not renew the account token");
            return None;
        }
    };
    match fetch_energy(&access).await {
        Ok(en) => {
            // The periodic poll and the UI's explicit refresh both look only for
            // recovery; a positive balance emits Resume after an observed 402.
            let persisted = {
                let _guard = token_refresh_lock().lock().await;
                let mut store = Credentials::load(data_dir);
                let current_access = store
                    .tokens
                    .as_ref()
                    .map(|t| t.access_token.trim())
                    .unwrap_or_default();
                if current_access != access.as_str() {
                    false
                } else {
                    store.energy = Some(en.clone());
                    match store.save(data_dir) {
                        Ok(()) => true,
                        Err(e) => {
                            tracing::debug!(error = %format!("{e:#}"), "failed to persist energy poll");
                            false
                        }
                    }
                }
            };
            if !persisted {
                return None;
            }
            crate::foundation::energy_state::reconcile(data_dir, en.remaining, en.total);
            // Every observed balance is also a point on the day's curve. This is the
            // frequent poll, so it is what gives the series its resolution.
            crate::foundation::energy_history::record(data_dir, &en);
            // Keeps the Settings page's "last checked" fresh between full refreshes.
            record_status(data_dir, true, "");
            Some(en)
        }
        Err(e) => {
            tracing::debug!(error = %format!("{e:#}"), "energy poll failed; keeping last value");
            None
        }
    }
}

/// Fire-and-forget energy poll for the periodic refresh loop.
async fn poll_energy(data_dir: &Path) {
    let _ = poll_energy_now(data_dir).await;
}

/// Spawn a detached loop that keeps managed credentials fresh while running: the
/// full configs refresh on a slow cadence (which also rotates the access token)
/// and an energy poll on a fast one. No-op in BYOK (each call returns early).
/// Best-effort; never panics.
pub fn spawn_refresh_loop(data_dir: std::path::PathBuf) {
    tokio::spawn(async move {
        let mut configs = tokio::time::interval(Duration::from_secs(3600));
        let mut energy = tokio::time::interval(Duration::from_secs(60));
        // Startup already refreshed once; drop the immediate first ticks.
        configs.tick().await;
        energy.tick().await;
        loop {
            tokio::select! {
                _ = configs.tick() => refresh(&data_dir, None).await,
                _ = energy.tick() => poll_energy(&data_dir).await,
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text_generation_configs() -> ConfigsDto {
        serde_json::from_value(serde_json::json!({
            "text-generation": {
                "anthropic-messages": {
                    "url": "https://songguo.example/v1/messages",
                    "api_key": "tok",
                    "models": [{
                        "model": "claude-opus-4-8",
                        "small": "claude-haiku-4-5-20251001",
                        "quality": 95
                    }]
                },
                "openai-responses": {
                    "url": "https://songguo.example/v1/responses",
                    "api_key": "tok",
                    "models": [
                        { "model": "gpt-5.5", "quality": 96 },
                        { "model": "gpt-5.4-mini", "quality": 78 }
                    ]
                }
            }
        }))
        .unwrap()
    }

    #[test]
    fn managed_from_takes_the_responses_wire() {
        let configs: ConfigsDto = serde_json::from_value(serde_json::json!({
            "text-generation": {
                "openai-responses": {
                    "url": "https://songguo.example/v1/responses",
                    "api_key": "tok",
                    "models": [
                        { "model": "gpt-5.5", "quality": 96 },
                        { "model": "gpt-5.4-mini", "quality": 78 }
                    ]
                }
            }
        }))
        .unwrap();

        let managed = managed_from(&configs);
        assert_eq!(managed.llm.wire, "openai-responses");
        // The endpoint leaf is stripped; the `/v1` prefix is part of the provider base
        // codex appends `/responses` to.
        assert_eq!(managed.llm.base_url, "https://songguo.example/v1");
        assert_eq!(managed.llm.api_key, "tok");
        assert_eq!(managed.llm.model.as_deref(), Some("gpt-5.5"));
        assert_eq!(managed.llm.small, None);
    }

    /// The broker still lists `anthropic-messages` for installs that used to drive
    /// Claude Code. We drive codex now, so that wire is ignored rather than preferred.
    #[test]
    fn managed_from_ignores_the_anthropic_wire() {
        let managed = managed_from(&text_generation_configs());
        assert_eq!(managed.llm.wire, "openai-responses");
        assert_eq!(managed.llm.base_url, "https://songguo.example/v1");
        assert_eq!(managed.llm.model.as_deref(), Some("gpt-5.5"));
    }

    /// A broker old enough to offer *only* the Anthropic wire leaves the agent
    /// unconfigured, which is the honest outcome: there is nothing here codex can speak,
    /// and booting against it would fail at the first turn instead of at startup.
    #[test]
    fn an_anthropic_only_broker_leaves_the_llm_unconfigured() {
        let configs: ConfigsDto = serde_json::from_value(serde_json::json!({
            "text-generation": {
                "anthropic-messages": {
                    "url": "https://songguo.example/v1/messages",
                    "api_key": "tok",
                    "models": [{ "model": "claude-opus-4-8", "quality": 95 }]
                }
            }
        }))
        .unwrap();

        let managed = managed_from(&configs);
        assert_eq!(managed.llm.wire, "");
        assert_eq!(managed.llm.api_key, "");
    }

    /// **The regression this shape exists for.** songguo served `text-to-image` over two
    /// wires — seedream on `openai-images`, `gpt-image-2` on `openai-responses` — and the
    /// old collapse kept whichever wire sorted first *by name*, so `gpt-image-2` was
    /// invisible to the agent and nothing said so. Every wire now survives the trip.
    #[test]
    fn every_wire_a_task_offers_survives() {
        let configs: ConfigsDto = serde_json::from_value(serde_json::json!({
            "text-to-image": {
                "openai-images": {
                    "url": "https://songguo.example/v1/images/generations",
                    "api_key": "tok",
                    "models": [{ "model": "doubao-seedream-5.0-lite", "quality": 75 }]
                },
                "openai-responses": {
                    "url": "https://songguo.example/v1/responses",
                    "api_key": "tok",
                    "models": [{ "model": "gpt-image-2", "quality": 96 }]
                }
            }
        }))
        .unwrap();

        let image = managed_from(&configs).image;
        assert_eq!(image.len(), 2, "neither wire may be dropped");
        // Ranked by the best model on offer, not by the wire's name — `openai-images`
        // sorts first alphabetically and would have won under the old rule.
        assert_eq!(image[0].wire, "openai-responses");
        assert_eq!(image[0].model.as_deref(), Some("gpt-image-2"));
        assert_eq!(image[0].base_url, "https://songguo.example/v1/responses");
        assert_eq!(image[1].wire, "openai-images");
        assert_eq!(image[1].model.as_deref(), Some("doubao-seedream-5.0-lite"));
        // Each wire keeps its own menu; the lists are not merged.
        assert_eq!(image[0].models.len(), 1);
        assert_eq!(image[1].models[0].name, "doubao-seedream-5.0-lite");
    }

    /// The wire id reaches the capability now — it used to be blanked here because
    /// these capabilities treated an unfamiliar spelling as fatal, and they no longer do.
    #[test]
    fn a_single_wire_task_still_carries_its_id_and_menu() {
        let configs: ConfigsDto = serde_json::from_value(serde_json::json!({
            "automatic-speech-recognition": {
                "volc-asr-stream-async": {
                    "url": "https://songguo.example/v1/asr",
                    "api_key": "tok",
                    "models": [{ "model": "bigmodel-asr", "quality": 80 }]
                }
            }
        }))
        .unwrap();

        let stt = managed_from(&configs).stt;
        assert_eq!(stt.len(), 1);
        assert_eq!(stt[0].wire, "volc-asr-stream-async");
        assert_eq!(stt[0].model.as_deref(), Some("bigmodel-asr"));
    }

    /// A task the broker does not offer at all leaves an empty list, not a phantom
    /// provider with a blank key.
    #[test]
    fn an_unoffered_task_is_empty() {
        assert!(managed_from(&text_generation_configs()).video.is_empty());
    }

    #[test]
    fn account_access_token_is_reused_only_with_a_safety_window() {
        let tokens = |expires_at: chrono::DateTime<chrono::Utc>| Tokens {
            access_token: "access".to_string(),
            refresh_token: "refresh".to_string(),
            access_expires_at: expires_at.to_rfc3339(),
        };

        assert!(
            fresh_access_token(
                &tokens(chrono::Utc::now() + chrono::Duration::minutes(10)),
                ACCOUNT_TOKEN_MIN_VALIDITY,
            )
            .is_some()
        );
        assert!(
            fresh_access_token(
                &tokens(chrono::Utc::now() + chrono::Duration::minutes(1)),
                ACCOUNT_TOKEN_MIN_VALIDITY,
            )
            .is_none()
        );
    }
}
