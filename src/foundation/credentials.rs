//! Credential store: the user's BYOK keys, or (xiaoyuanzhu) the broker-issued
//! account tokens plus the configs the broker hands back. Persisted under the
//! data dir as `config.db` (SQLite; see the [`db`] submodule), resolved at
//! startup, refreshed by the broker client. There is no `.env` credential
//! fallback: the default `xiaoyuanzhu` mode auto-bootstraps a broker account and
//! mints the keys OOTB, and BYOK keys are entered in Settings. A vendor key in
//! effect also implies that vendor is the provider for its capability; a capability
//! with no key is simply off.
//!
//! Both modes' configs are stored side by side (one row per `(mode, feature, wire)`),
//! so switching mode in Settings surfaces whatever was last entered for it, and a
//! capability served over several wires keeps all of them. A legacy
//! `credentials.json` is imported once on first load (see [`db::load`]).

use std::path::{Path, PathBuf};

use anyhow::Context;
use serde::{Deserialize, Serialize};

/// File under the data dir holding the credential store (SQLite). Named in
/// `hi-wire` because the roster shares it — an app that hosts a core writes to
/// the same file, and an app with no core owns it alone.
use hi_wire::STORE_FILE as FILE;

/// `app_settings` key recording the port the local HTTP server bound this run, so
/// the native Settings "Sign in" button and the account-link handlers can build the
/// loopback callback URL. Written at startup; not a secret.
pub const KEY_SERVER_PORT: &str = "server_port";

/// Absolute path to the credential store for `data_dir`.
pub fn path(data_dir: &Path) -> PathBuf {
    data_dir.join(FILE)
}

/// A single app-level setting (the `app_settings` KV table) — e.g. a cognition
/// tunable. `None` when absent, blank, or the store can't be read. This is the same
/// table the credential `mode`/`device_id` live in; callers key their own namespace.
pub fn get_setting(data_dir: &Path, key: &str) -> Option<String> {
    let v = db::get_setting_at(data_dir, key).ok().flatten()?;
    let v = v.trim();
    (!v.is_empty()).then(|| v.to_string())
}

/// Set a single app-level setting (upsert). Used by the settings handler to persist
/// the cognition tunables the UI edits.
pub fn set_setting(data_dir: &Path, key: &str, value: &str) -> anyhow::Result<()> {
    db::set_setting_at(data_dir, key, value)
}

/// Every app-level setting as a map — the startup snapshot the reaction's tunables
/// global loads. Empty on a fresh / unreadable store (defaults then apply).
pub fn all_settings(data_dir: &Path) -> std::collections::HashMap<String, String> {
    db::all_settings(data_dir).unwrap_or_default()
}

/// Append one observed balance to the sample log and prune past `keep_since`. The
/// retention window and what counts as worth recording are policy, and live in
/// [`crate::foundation::energy_history`]; this is only the store.
pub fn record_energy_sample(
    data_dir: &Path,
    at: &str,
    remaining: i64,
    total: i64,
    tier: &str,
    keep_since: &str,
) -> anyhow::Result<()> {
    db::record_energy_sample(data_dir, at, remaining, total, tier, keep_since)
}

/// Recorded `(at, remaining, total)` samples at or after `since`, oldest first.
pub fn energy_samples_since(data_dir: &Path, since: &str) -> anyhow::Result<Vec<(String, i64, i64)>> {
    db::energy_samples_since(data_dir, since)
}

/// Parse a stored mode string, case-insensitive (`byok` | `xiaoyuanzhu`). The
/// legacy values `free`/`login` map to `xiaoyuanzhu` (the mode that absorbed
/// both). Unknown → None. The Settings UI / config store is the sole authority for
/// the mode; there is no env override.
fn parse_mode(s: &str) -> Option<Mode> {
    match s.trim().to_ascii_lowercase().as_str() {
        "byok" => Some(Mode::Byok),
        "xiaoyuanzhu" | "free" | "login" => Some(Mode::Xiaoyuanzhu),
        _ => None,
    }
}

/// How the agent obtains its credentials.
/// - `xiaoyuanzhu`: a broker account (`hi-agent.xyz`) — the default, so a
///   fresh install works with no setup. Anonymous device bootstrap yields the
///   `free` tier; a signed-in account.xiaoyuanzhu.com session yields `sub`.
/// - `byok`: the user's own keys (the flat fields below).
///
/// `xiaoyuanzhu` goes through a one-time **bootstrap** that yields account
/// [`Tokens`]; the access token then authenticates the configs + energy fetches.
///
/// The legacy `free`/`login` values deserialize to `Xiaoyuanzhu` (they were split
/// modes that collapsed into it), so an older `credentials.json` loads unchanged.
#[derive(Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, Debug)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    Byok,
    #[default]
    #[serde(alias = "free", alias = "login")]
    Xiaoyuanzhu,
}

/// The user's credentials (BYOK) plus, for xiaoyuanzhu, the broker account tokens
/// and the configs/energy the broker minted. [`Credentials::effective`] picks
/// which credential set is live for the current mode.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Credentials {
    /// Which credential source is live. Default `xiaoyuanzhu`.
    pub mode: Mode,
    pub llm: LlmCredentials,
    pub stt: VendorKey,
    pub tts: VendorKey,
    pub vision: VendorKey,
    pub image: VendorKey,
    pub video: VendorKey,
    /// Stable per-install id — the seed for the free bootstrap (not a secret).
    #[serde(skip_serializing_if = "String::is_empty")]
    pub device_id: String,
    /// Broker-issued account tokens (xiaoyuanzhu). The unified credential after
    /// bootstrap: the access token authenticates configs + energy; the refresh
    /// token mints a new access when it expires.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tokens: Option<Tokens>,
    /// Last configs the broker minted (xiaoyuanzhu) — the vendor settings applied.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub managed: Option<Managed>,
    /// Last energy snapshot, for the Settings bar. Polled on its own cadence,
    /// separate from configs.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub energy: Option<Energy>,
    /// The signed-in web identity after a device claim (web→device link). `None`
    /// while anonymous. Display only — the account is keyed by [`Tokens`], not this;
    /// Settings shows it as "signed in as …".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identity: Option<Identity>,
}

/// The bound web account's display identity, recorded when this device is linked to
/// it (see `broker::claim_device`). Not a credential — the tokens are the account.
#[derive(Clone, Default, Serialize, Deserialize, Debug)]
#[serde(default)]
pub struct Identity {
    pub email: String,
    pub name: String,
    /// The account tier at claim time (e.g. `standard`/`pro`/`max`). Advisory label.
    pub tier: String,
}

/// Broker-issued account tokens. The access token is a short-lived bearer for
/// configs/energy; the refresh token mints new access tokens (and is rotated each
/// refresh, so the newest must always be persisted).
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Tokens {
    pub access_token: String,
    pub refresh_token: String,
    /// RFC3339 access-token expiry; refresh at or before this.
    pub access_expires_at: String,
}

/// Upstream LLM credentials — held by the trusted host's privacy proxy. Codex receives
/// the model choice plus a loopback provider and never receives this endpoint or key.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct LlmCredentials {
    /// Which wire (vendor/protocol impl) backs the LLM; empty → the feature's
    /// default. Reserved for a future second LLM wire; `resolve` drives the single
    /// one (codex over OpenAI Responses) today. See [`VendorKey::wire`].
    #[serde(skip_serializing_if = "String::is_empty")]
    pub wire: String,
    /// Upstream base URL — the provider *base* codex appends `/responses` to; empty
    /// → the built-in default.
    pub base_url: String,
    /// Upstream API key; empty → not configured (falls back to `.env`).
    pub api_key: String,
    /// Model override; `None` → let codex pick its own default.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Cheaper/faster companion model for background work; `None` → reuse `model`.
    /// Only the managed (broker) path sets this today.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub small: Option<String>,
}

impl LlmCredentials {
    /// The configured wire id if set, else `None` (use the feature default).
    pub fn wire_opt(&self) -> Option<&str> {
        let w = self.wire.trim();
        if w.is_empty() { None } else { Some(w) }
    }
}

/// **One wire's** config for one capability. In BYOK only `api_key` is set (other
/// params stay on env defaults); in managed mode the broker also fills `base_url`
/// (songguo) and may fill `model`, and the vendor host-rebases its native endpoint
/// onto songguo.
///
/// `wire` names which vendor/protocol impl serves the feature. A capability holds a
/// *list* of these — one per wire the source offers (see [`Managed`]) — and the
/// capability's `init` dispatches on the id. Empty means the feature's default wire.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct VendorKey {
    /// Which wire (vendor impl) backs this feature; empty → the feature default.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub wire: String,
    /// Gateway base; empty → the vendor's own default endpoint.
    pub base_url: String,
    pub api_key: String,
    /// Model override; None → the vendor's default.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// The mainline model that carries a hosted tool on wires where the work is done
    /// *inside a turn* rather than by its own endpoint — today only `openai-responses`
    /// for `text-to-image`, where the picture comes from an `image_generation` tool.
    ///
    /// A property of the wire, not of a model: it says which model hosts the turn, and
    /// the models list still names what the agent may draw with. Empty everywhere else,
    /// and never defaulted — a carrier we picked would bill for a choice nobody made.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub carrier: String,
    /// Every model this endpoint serves, as the broker published them. **Empty under
    /// BYOK**, where there is no menu — only the key its owner pasted.
    ///
    /// Here because the generation capabilities let the *agent* name a model, and an
    /// agent can only choose from a list it has been shown. `model` above stays what
    /// it was: the default when nobody names one.
    ///
    /// Not carried by the SQLite credential store (no column), only the JSON one. That
    /// costs nothing: [`crate::foundation::broker::refresh`] runs at boot and refills
    /// it, and a boot that cannot reach the broker degrades to pass-through — any
    /// model name still reaches the sole configured provider.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub models: Vec<ModelOffer>,
}

/// One model on a broker-published menu, with its relative hints. Kept in the
/// credential vocabulary rather than a capability's, so `credentials` stays free of
/// any dependency on `image_gen`/`video_gen`; each capability maps this into its own
/// type at the composition root.
#[derive(Clone, Default, Serialize, Deserialize, Debug, PartialEq, Eq)]
#[serde(default)]
pub struct ModelOffer {
    pub name: String,
    pub quality: i64,
    pub speed: i64,
    pub price: i64,
}

impl VendorKey {
    /// The trimmed key if non-empty, else `None` — the "use my key / fall back to
    /// env" signal threaded into each capability's init.
    pub fn key_opt(&self) -> Option<&str> {
        let k = self.api_key.trim();
        if k.is_empty() { None } else { Some(k) }
    }

    /// The managed gateway base if set, else `None` (use the vendor's default).
    pub fn base_url_opt(&self) -> Option<&str> {
        let b = self.base_url.trim();
        if b.is_empty() { None } else { Some(b) }
    }

    /// The managed model override if set, else `None`.
    pub fn model_opt(&self) -> Option<&str> {
        self.model.as_deref().map(str::trim).filter(|m| !m.is_empty())
    }

    /// The tool-carrying mainline model if set, else `None`.
    pub fn carrier_opt(&self) -> Option<&str> {
        let c = self.carrier.trim();
        if c.is_empty() { None } else { Some(c) }
    }

    /// The configured wire id if set, else `None` (use the feature default).
    pub fn wire_opt(&self) -> Option<&str> {
        let w = self.wire.trim();
        if w.is_empty() { None } else { Some(w) }
    }
}

/// Broker-minted configs (xiaoyuanzhu): the same credential fields as BYOK. The
/// account/energy snapshot is separate ([`Energy`]) so it can be polled often
/// without re-fetching configs.
///
/// **Every capability slot is a list, because the broker's menu is a list.** Its
/// `/configs` is task → *wire* → endpoint, and songguo routinely offers a task over
/// more than one wire (`text-to-image` over both `openai-images` and `openai-responses`;
/// `text-generation` over both `openai-responses` and `anthropic-messages`). Collapsing
/// that to one wire per task threw away every model served over the others — silently,
/// since the survivor was whichever wire sorted first by name. The capability, which is
/// the only layer that knows which shapes it can actually speak, now decides.
///
/// The LLM stays singular: codex is the agent runtime and it speaks OpenAI Responses,
/// so a second LLM wire is a second engine, not a second config. See
/// [`crate::foundation::broker::pick_llm_wire`].
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Managed {
    pub llm: LlmCredentials,
    #[serde(deserialize_with = "one_or_many")]
    pub stt: Vec<VendorKey>,
    #[serde(deserialize_with = "one_or_many")]
    pub tts: Vec<VendorKey>,
    #[serde(deserialize_with = "one_or_many")]
    pub vision: Vec<VendorKey>,
    #[serde(deserialize_with = "one_or_many")]
    pub image: Vec<VendorKey>,
    #[serde(deserialize_with = "one_or_many")]
    pub video: Vec<VendorKey>,
}

/// Accept both a bare `VendorKey` object and an array of them.
///
/// The slots were single objects before multi-wire, and a stored `credentials.json`
/// from that era is still imported on first load ([`db::load`]). A hard parse error
/// there does not lose one slot — serde fails the whole `Credentials`, so a legacy file
/// would take the account and the BYOK keys down with it.
fn one_or_many<'de, D>(d: D) -> Result<Vec<VendorKey>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum OneOrMany {
        One(Box<VendorKey>),
        Many(Vec<VendorKey>),
    }
    Ok(match OneOrMany::deserialize(d)? {
        OneOrMany::One(v) => vec![*v],
        OneOrMany::Many(v) => v,
    })
}

/// The user-facing balance from `/energy` (xiaoyuanzhu). Cached for display; the
/// live value is metered at the gateway. `unit` is always "energy".
#[derive(Clone, Default, Serialize, Deserialize, Debug)]
#[serde(default)]
pub struct Energy {
    pub remaining: i64,
    pub total: i64,
    pub resets_at: String,
    /// Tier the broker reports, in its own vocabulary: `standard` (the included
    /// allowance every account gets) | `pro` | `max`. Carried through as text — the
    /// broker owns this list, and a tier we don't recognize must still display.
    pub tier: String,
}

/// The credentials in effect for the current mode — borrows from either the BYOK
/// fields or the managed configs.
///
/// Each capability slot is a slice of the wires configured for it, best first. BYOK
/// yields exactly one — a person pastes one key per capability, and there is no menu
/// to choose from — so the slice is the shape both modes share rather than a shape
/// invented for the broker.
pub struct Effective<'a> {
    pub llm: &'a LlmCredentials,
    pub stt: &'a [VendorKey],
    pub tts: &'a [VendorKey],
    pub vision: &'a [VendorKey],
    pub image: &'a [VendorKey],
    pub video: &'a [VendorKey],
}

impl Credentials {
    /// The credentials in effect: BYOK fields in `byok` mode, the managed configs
    /// in xiaoyuanzhu. `None` in xiaoyuanzhu before configs have been fetched —
    /// callers then fall back to `.env` (resolve) or leave the capability off.
    pub fn effective(&self) -> Option<Effective<'_>> {
        match self.mode {
            Mode::Byok => Some(Effective {
                llm: &self.llm,
                stt: std::slice::from_ref(&self.stt),
                tts: std::slice::from_ref(&self.tts),
                vision: std::slice::from_ref(&self.vision),
                image: std::slice::from_ref(&self.image),
                video: std::slice::from_ref(&self.video),
            }),
            Mode::Xiaoyuanzhu => self.managed.as_ref().map(|m| Effective {
                llm: &m.llm,
                stt: &m.stt,
                tts: &m.tts,
                vision: &m.vision,
                image: &m.image,
                video: &m.video,
            }),
        }
    }

    /// Load from `<data_dir>/config.db`. A missing DB yields defaults; any read
    /// error logs a warning and also yields defaults, so a corrupt store can't
    /// brick boot — the user re-saves from Settings. On first load a legacy
    /// `credentials.json` is imported into the DB (see [`db::load`]).
    pub fn load(data_dir: &Path) -> Self {
        Self::try_load(data_dir).unwrap_or_else(|e| {
            tracing::warn!(
                path = %path(data_dir).display(), error = %format!("{e:#}"),
                "config store unreadable; using defaults (re-save from Settings)"
            );
            Self::default()
        })
    }

    /// The same read, with the failure kept.
    ///
    /// [`load`](Self::load) turns any read error into [`Default`], and the default is
    /// `Xiaoyuanzhu` with no managed bundle — which is exactly what a machine that has
    /// never been configured looks like. A caller that can tell those two apart must
    /// have the error, because the value cannot: on 2026-08-29 a read that lost a race
    /// with the broker's own writer handed the agent a default config, and a codex child
    /// was spawned against `api.openai.com` with an empty key. Callers who genuinely do
    /// not care (Settings, the API handlers) keep using `load`.
    pub fn try_load(data_dir: &Path) -> anyhow::Result<Self> {
        db::load(data_dir)
    }

    /// Persist to `<data_dir>/config.db`, owner-only (`0600` on unix). Writes both
    /// modes' rows so a later mode switch surfaces the stored config for it.
    pub fn save(&self, data_dir: &Path) -> anyhow::Result<()> {
        db::save(data_dir, self)
    }
}

fn redact(s: &str) -> &'static str {
    if s.trim().is_empty() { "<unset>" } else { "<redacted>" }
}

// Hand-written Debug impls so a stray trace never prints a secret.
impl std::fmt::Debug for LlmCredentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LlmCredentials")
            .field("base_url", &self.base_url)
            .field("api_key", &redact(&self.api_key))
            .field("model", &self.model)
            .field("small", &self.small)
            .finish()
    }
}

impl std::fmt::Debug for VendorKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VendorKey")
            .field("base_url", &self.base_url)
            .field("api_key", &redact(&self.api_key))
            .field("model", &self.model)
            .finish()
    }
}

impl std::fmt::Debug for Tokens {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Tokens")
            .field("access_token", &redact(&self.access_token))
            .field("refresh_token", &redact(&self.refresh_token))
            .field("access_expires_at", &self.access_expires_at)
            .finish()
    }
}

impl std::fmt::Debug for Managed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Managed").field("llm", &self.llm).finish_non_exhaustive()
    }
}

impl std::fmt::Debug for Credentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Credentials")
            .field("mode", &self.mode)
            .field("llm", &self.llm)
            .field("stt", &self.stt)
            .field("tts", &self.tts)
            .field("vision", &self.vision)
            .field("image", &self.image)
            .field("video", &self.video)
            .field("device_id", &self.device_id)
            .field("tokens", &self.tokens)
            .field("managed", &self.managed)
            .field("energy", &self.energy)
            .finish()
    }
}

/// SQLite persistence for the credential store. The on-disk shape is normalized:
/// scalar flags in `app_settings`, one `credential` row per `(mode, feature)` (so
/// both modes coexist), and a single-row `account` for the broker tokens + energy.
/// The mapping to/from the in-memory [`Credentials`] lives entirely here; the rest
/// of the tree only sees [`Credentials::load`] / [`Credentials::save`].
mod db {
    use super::*;
    use rusqlite::{Connection, OptionalExtension, params};

    /// The broker account/energy row is a singleton (`id = 1`).
    const ACCOUNT_ID: i64 = 1;

    const SCHEMA: &str = "
        CREATE TABLE IF NOT EXISTS app_settings (
            key   TEXT PRIMARY KEY,
            value TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS credential (
            mode     TEXT NOT NULL,
            feature  TEXT NOT NULL,
            wire     TEXT NOT NULL DEFAULT '',
            base_url TEXT NOT NULL DEFAULT '',
            api_key  TEXT NOT NULL DEFAULT '',
            model    TEXT,
            small    TEXT,
            -- The broker's published menu for this endpoint, as a JSON array. A
            -- column rather than a derived-at-boot value because the fetch and the
            -- read are not the same moment: `broker::refresh` saves here and
            -- `capabilities::init` loads back, so anything not written is simply
            -- gone by the time the capability asks.
            models   TEXT,
            -- The mainline model that carries a hosted tool, on the wires that need
            -- one (openai-responses image generation). Empty on every other wire.
            carrier  TEXT,
            -- Keyed by wire as well as feature: one capability holds one row per wire
            -- its source offers, and the capability picks among them. See `Managed`.
            PRIMARY KEY (mode, feature, wire)
        );
        CREATE TABLE IF NOT EXISTS account (
            id                INTEGER PRIMARY KEY CHECK (id = 1),
            access_token      TEXT,
            refresh_token     TEXT,
            access_expires_at TEXT,
            energy_remaining  INTEGER,
            energy_total      INTEGER,
            energy_resets_at  TEXT,
            energy_tier       TEXT
        );
        -- One row per observed balance, so the level can be drawn over time. The
        -- `account` row above holds only the latest, which cannot answer 'where did
        -- today's energy go'. Keyed by the sample instant (RFC3339 UTC): a poll that
        -- lands in the same second is the same observation, not a second one.
        CREATE TABLE IF NOT EXISTS energy_sample (
            at        TEXT PRIMARY KEY,
            remaining INTEGER NOT NULL,
            total     INTEGER NOT NULL,
            tier      TEXT NOT NULL DEFAULT ''
        );
    ";

    /// The stable string a `Mode` is stored under (matches the serde/JSON name, so
    /// legacy imports and the wire API line up).
    fn mode_str(m: Mode) -> &'static str {
        match m {
            Mode::Byok => "byok",
            Mode::Xiaoyuanzhu => "xiaoyuanzhu",
        }
    }

    /// Open (creating if needed) the config DB, ensure the schema, and lock it down
    /// to owner-only. A short busy timeout lets the startup load, the settings
    /// writes, and the periodic broker poll serialize instead of erroring.
    fn open(data_dir: &Path) -> anyhow::Result<Connection> {
        std::fs::create_dir_all(data_dir)
            .with_context(|| format!("creating data dir {}", data_dir.display()))?;
        let p = path(data_dir);
        let conn = Connection::open(&p).with_context(|| format!("opening {}", p.display()))?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        conn.execute_batch(SCHEMA).context("initializing config schema")?;
        migrate(&conn).context("migrating config schema")?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o600));
        }
        Ok(conn)
    }

    /// Bring an older DB up to the current schema (the `CREATE TABLE IF NOT EXISTS`
    /// above already covers fresh DBs). Idempotent column adds, then the one
    /// non-additive step — widening the `credential` key to include `wire`. Each is
    /// guarded on `table_info` so re-running is a no-op.
    fn migrate(conn: &Connection) -> anyhow::Result<()> {
        if !column_exists(conn, "credential", "wire")? {
            conn.execute_batch("ALTER TABLE credential ADD COLUMN wire TEXT NOT NULL DEFAULT ''")?;
        }
        if !column_exists(conn, "credential", "small")? {
            conn.execute_batch("ALTER TABLE credential ADD COLUMN small TEXT")?;
        }
        if !column_exists(conn, "credential", "models")? {
            conn.execute_batch("ALTER TABLE credential ADD COLUMN models TEXT")?;
        }
        if !column_exists(conn, "credential", "carrier")? {
            conn.execute_batch("ALTER TABLE credential ADD COLUMN carrier TEXT")?;
        }
        widen_credential_key(conn)?;
        Ok(())
    }

    /// Re-key `credential` from `(mode, feature)` to `(mode, feature, wire)`.
    ///
    /// SQLite cannot alter a primary key, so this is the table-rebuild dance. Guarded
    /// on the live key, so it runs once and is a no-op forever after. No row is lost or
    /// changed: an old store held at most one row per `(mode, feature)`, which is
    /// exactly one row per `(mode, feature, wire)` too — and the broker refills the
    /// managed rows at boot anyway.
    fn widen_credential_key(conn: &Connection) -> anyhow::Result<()> {
        if key_includes_wire(conn)? {
            return Ok(());
        }
        conn.execute_batch(
            "BEGIN;
             CREATE TABLE credential_new (
                 mode     TEXT NOT NULL,
                 feature  TEXT NOT NULL,
                 wire     TEXT NOT NULL DEFAULT '',
                 base_url TEXT NOT NULL DEFAULT '',
                 api_key  TEXT NOT NULL DEFAULT '',
                 model    TEXT,
                 small    TEXT,
                 models   TEXT,
                 carrier  TEXT,
                 PRIMARY KEY (mode, feature, wire)
             );
             INSERT INTO credential_new (mode, feature, wire, base_url, api_key, model, small, models, carrier)
                 SELECT mode, feature, wire, base_url, api_key, model, small, models, carrier FROM credential;
             DROP TABLE credential;
             ALTER TABLE credential_new RENAME TO credential;
             COMMIT;",
        )?;
        Ok(())
    }

    /// Whether `wire` is part of the `credential` primary key — `table_info`'s `pk`
    /// column is 0 for a non-key column and its 1-based position otherwise.
    fn key_includes_wire(conn: &Connection) -> anyhow::Result<bool> {
        let mut stmt = conn.prepare("PRAGMA table_info(credential)")?;
        let mut rows = stmt.query([])?;
        while let Some(row) = rows.next()? {
            let name: String = row.get(1)?;
            let pk: i64 = row.get(5)?;
            if name == "wire" {
                return Ok(pk > 0);
            }
        }
        Ok(false)
    }

    fn column_exists(conn: &Connection, table: &str, column: &str) -> anyhow::Result<bool> {
        let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
        let mut rows = stmt.query([])?;
        while let Some(row) = rows.next()? {
            let name: String = row.get(1)?; // (cid, name, type, ...)
            if name == column {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Load the credential store, importing a legacy `credentials.json` on first run.
    pub fn load(data_dir: &Path) -> anyhow::Result<Credentials> {
        let conn = open(data_dir)?;
        maybe_import_legacy(&conn, data_dir)?;
        read(&conn)
    }

    /// Persist the whole store atomically (both modes' rows + the account).
    pub fn save(data_dir: &Path, c: &Credentials) -> anyhow::Result<()> {
        let conn = open(data_dir)?;
        let tx = conn.unchecked_transaction()?;
        write_all(&tx, c)?;
        tx.commit()?;
        Ok(())
    }

    /// Reconstruct a [`Credentials`] from the tables. Absent rows read as empty /
    /// `None`, so a partially-populated store loads cleanly.
    fn read(conn: &Connection) -> anyhow::Result<Credentials> {
        let mode = get_setting(conn, "mode")?
            .and_then(|s| parse_mode(&s))
            .unwrap_or_default();
        let device_id = get_setting(conn, "device_id")?.unwrap_or_default();
        let (tokens, energy) = read_account(conn)?;
        Ok(Credentials {
            mode,
            device_id,
            llm: read_llm(conn, Mode::Byok, "llm")?,
            stt: read_vendor(conn, Mode::Byok, "stt")?,
            tts: read_vendor(conn, Mode::Byok, "tts")?,
            vision: read_vendor(conn, Mode::Byok, "vision")?,
            image: read_vendor(conn, Mode::Byok, "image")?,
            video: read_vendor(conn, Mode::Byok, "video")?,
            managed: read_managed(conn)?,
            tokens,
            energy,
            identity: read_identity(conn)?,
        })
    }

    /// The bound identity, from its `app_settings` scalars. `None` unless an email
    /// is stored (an anonymous device account leaves them blank).
    fn read_identity(conn: &Connection) -> anyhow::Result<Option<Identity>> {
        let email = get_setting(conn, "identity_email")?.unwrap_or_default();
        if email.trim().is_empty() {
            return Ok(None);
        }
        Ok(Some(Identity {
            email,
            name: get_setting(conn, "identity_name")?.unwrap_or_default(),
            tier: get_setting(conn, "identity_tier")?.unwrap_or_default(),
        }))
    }

    /// Persist (or clear) the bound identity scalars.
    fn write_identity(conn: &Connection, id: Option<&Identity>) -> anyhow::Result<()> {
        let (email, name, tier) = match id {
            Some(i) => (i.email.as_str(), i.name.as_str(), i.tier.as_str()),
            None => ("", "", ""),
        };
        set_setting(conn, "identity_email", email)?;
        set_setting(conn, "identity_name", name)?;
        set_setting(conn, "identity_tier", tier)?;
        Ok(())
    }

    /// Write every field of `c` — the BYOK flat fields, the managed bundle (when
    /// present), the account, and the scalar flags. Upserts, so re-saving is idempotent.
    fn write_all(conn: &Connection, c: &Credentials) -> anyhow::Result<()> {
        set_setting(conn, "mode", mode_str(c.mode))?;
        set_setting(conn, "device_id", &c.device_id)?;
        write_llm(conn, Mode::Byok, "llm", &c.llm)?;
        write_vendors(conn, Mode::Byok, "stt", std::slice::from_ref(&c.stt))?;
        write_vendors(conn, Mode::Byok, "tts", std::slice::from_ref(&c.tts))?;
        write_vendors(conn, Mode::Byok, "vision", std::slice::from_ref(&c.vision))?;
        write_vendors(conn, Mode::Byok, "image", std::slice::from_ref(&c.image))?;
        write_vendors(conn, Mode::Byok, "video", std::slice::from_ref(&c.video))?;
        if let Some(m) = &c.managed {
            write_llm(conn, Mode::Xiaoyuanzhu, "llm", &m.llm)?;
            write_vendors(conn, Mode::Xiaoyuanzhu, "stt", &m.stt)?;
            write_vendors(conn, Mode::Xiaoyuanzhu, "tts", &m.tts)?;
            write_vendors(conn, Mode::Xiaoyuanzhu, "vision", &m.vision)?;
            write_vendors(conn, Mode::Xiaoyuanzhu, "image", &m.image)?;
            write_vendors(conn, Mode::Xiaoyuanzhu, "video", &m.video)?;
        }
        write_account(conn, c)?;
        write_identity(conn, c.identity.as_ref())?;
        Ok(())
    }

    fn get_setting(conn: &Connection, key: &str) -> anyhow::Result<Option<String>> {
        Ok(conn
            .query_row("SELECT value FROM app_settings WHERE key = ?1", params![key], |r| r.get(0))
            .optional()?)
    }

    fn set_setting(conn: &Connection, key: &str, value: &str) -> anyhow::Result<()> {
        conn.execute(
            "INSERT INTO app_settings (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }

    /// Read one setting by key, opening the store directly (for the pub `get_setting`
    /// accessor — a caller that has only a `data_dir`, not a live connection).
    pub fn get_setting_at(data_dir: &Path, key: &str) -> anyhow::Result<Option<String>> {
        get_setting(&open(data_dir)?, key)
    }

    /// Upsert one setting by key, opening the store directly (for the pub accessor).
    pub fn set_setting_at(data_dir: &Path, key: &str, value: &str) -> anyhow::Result<()> {
        set_setting(&open(data_dir)?, key, value)
    }

    /// Every `app_settings` row as a map, opening the store directly.
    pub fn all_settings(data_dir: &Path) -> anyhow::Result<std::collections::HashMap<String, String>> {
        let conn = open(data_dir)?;
        let mut stmt = conn.prepare("SELECT key, value FROM app_settings")?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
        let mut map = std::collections::HashMap::new();
        for row in rows {
            let (k, v) = row?;
            map.insert(k, v);
        }
        Ok(map)
    }

    /// Append one observed balance and drop everything older than `keep_since`, so
    /// the table stays bounded by the retention window rather than by uptime.
    pub fn record_energy_sample(
        data_dir: &Path,
        at: &str,
        remaining: i64,
        total: i64,
        tier: &str,
        keep_since: &str,
    ) -> anyhow::Result<()> {
        let conn = open(data_dir)?;
        conn.execute(
            "INSERT INTO energy_sample (at, remaining, total, tier) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(at) DO UPDATE SET
                 remaining = excluded.remaining,
                 total     = excluded.total,
                 tier      = excluded.tier",
            params![at, remaining, total, tier],
        )?;
        conn.execute("DELETE FROM energy_sample WHERE at < ?1", params![keep_since])?;
        Ok(())
    }

    /// Every sample at or after `since`, oldest first.
    pub fn energy_samples_since(
        data_dir: &Path,
        since: &str,
    ) -> anyhow::Result<Vec<(String, i64, i64)>> {
        let conn = open(data_dir)?;
        let mut stmt = conn.prepare(
            "SELECT at, remaining, total FROM energy_sample WHERE at >= ?1 ORDER BY at ASC",
        )?;
        let rows = stmt.query_map(params![since], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?, r.get::<_, i64>(2)?))
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    /// The `(wire, base_url, api_key, model, small)` tuple for one `(mode, feature)`,
    /// or `None` when no row exists. `small` is only meaningful for the llm feature.
    fn read_row(
        conn: &Connection,
        mode: Mode,
        feature: &str,
    ) -> anyhow::Result<Option<(String, String, String, Option<String>, Option<String>)>> {
        Ok(conn
            .query_row(
                "SELECT wire, base_url, api_key, model, small FROM credential WHERE mode = ?1 AND feature = ?2",
                params![mode_str(mode), feature],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            )
            .optional()?)
    }

    /// Every wire stored for one `(mode, feature)`, in the order they were written —
    /// which is the order the source ranked them, so the first is the best.
    ///
    /// `rowid` is the tiebreaker rather than the wire name: the whole point of the
    /// change is that a wire's *name* must not decide anything, and inserting in rank
    /// order is how the ranking survives a round-trip through a table with no rank
    /// column. Malformed menu JSON reads as *no menu*, never as a failed load — the
    /// menu is a hint for choosing a model, the key beside it is what makes the
    /// capability work at all, and a garbled hint must not take the key down with it.
    /// The BYOK reader: one key per capability, because one person pasted it. Extra
    /// rows can only come from a mode that does not write them, so the first wins.
    fn read_vendor(conn: &Connection, mode: Mode, feature: &str) -> anyhow::Result<VendorKey> {
        Ok(read_vendors(conn, mode, feature)?.into_iter().next().unwrap_or_default())
    }

    fn read_vendors(conn: &Connection, mode: Mode, feature: &str) -> anyhow::Result<Vec<VendorKey>> {
        let mut stmt = conn.prepare(
            "SELECT wire, base_url, api_key, model, models, carrier FROM credential
             WHERE mode = ?1 AND feature = ?2 ORDER BY rowid",
        )?;
        let rows = stmt.query_map(params![mode_str(mode), feature], |r| {
            Ok(VendorKey {
                wire: r.get(0)?,
                base_url: r.get(1)?,
                api_key: r.get(2)?,
                model: r.get(3)?,
                models: r
                    .get::<_, Option<String>>(4)?
                    .and_then(|s| serde_json::from_str(&s).ok())
                    .unwrap_or_default(),
                carrier: r.get::<_, Option<String>>(5)?.unwrap_or_default(),
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    fn read_llm(conn: &Connection, mode: Mode, feature: &str) -> anyhow::Result<LlmCredentials> {
        Ok(read_row(conn, mode, feature)?
            .map(|(wire, base_url, api_key, model, small)| LlmCredentials {
                wire,
                base_url,
                api_key,
                model,
                small,
            })
            .unwrap_or_default())
    }

    /// The managed bundle is `Some` iff at least one xiaoyuanzhu row was stored —
    /// mirrors the JSON store's `managed: Option<Managed>` (absent until fetched).
    fn read_managed(conn: &Connection) -> anyhow::Result<Option<Managed>> {
        let any: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM credential WHERE mode = ?1)",
            params![mode_str(Mode::Xiaoyuanzhu)],
            |r| r.get(0),
        )?;
        if !any {
            return Ok(None);
        }
        Ok(Some(Managed {
            llm: read_llm(conn, Mode::Xiaoyuanzhu, "llm")?,
            stt: read_vendors(conn, Mode::Xiaoyuanzhu, "stt")?,
            tts: read_vendors(conn, Mode::Xiaoyuanzhu, "tts")?,
            vision: read_vendors(conn, Mode::Xiaoyuanzhu, "vision")?,
            image: read_vendors(conn, Mode::Xiaoyuanzhu, "image")?,
            video: read_vendors(conn, Mode::Xiaoyuanzhu, "video")?,
        }))
    }

    /// Replace every wire stored for one `(mode, feature)`.
    ///
    /// Delete-then-insert, not upsert: a wire the source has *stopped* offering has to
    /// disappear, and an upsert keyed on the wire can only ever add. Insert order is
    /// the rank [`read_vendors`] reads back.
    fn write_vendors(
        conn: &Connection,
        mode: Mode,
        feature: &str,
        vks: &[VendorKey],
    ) -> anyhow::Result<()> {
        clear_feature(conn, mode, feature)?;
        for vk in vks {
            let models =
                if vk.models.is_empty() { None } else { Some(serde_json::to_string(&vk.models)?) };
            conn.execute(
                "INSERT INTO credential (mode, feature, wire, base_url, api_key, model, small, models, carrier)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL, ?7, ?8)
                 ON CONFLICT(mode, feature, wire) DO UPDATE SET
                     base_url = excluded.base_url, api_key = excluded.api_key,
                     model = excluded.model, models = excluded.models,
                     carrier = excluded.carrier",
                params![
                    mode_str(mode),
                    feature,
                    vk.wire,
                    vk.base_url,
                    vk.api_key,
                    vk.model.as_deref(),
                    models,
                    vk.carrier_opt()
                ],
            )?;
        }
        Ok(())
    }

    fn clear_feature(conn: &Connection, mode: Mode, feature: &str) -> anyhow::Result<()> {
        conn.execute(
            "DELETE FROM credential WHERE mode = ?1 AND feature = ?2",
            params![mode_str(mode), feature],
        )?;
        Ok(())
    }

    fn write_llm(conn: &Connection, mode: Mode, feature: &str, llm: &LlmCredentials) -> anyhow::Result<()> {
        // Cleared first for the same reason as [`write_vendors`]: with `wire` in the
        // key, changing wires under an upsert leaves the old row behind as a second
        // answer to a question that has one.
        clear_feature(conn, mode, feature)?;
        write_row(
            conn,
            mode,
            feature,
            &llm.wire,
            &llm.base_url,
            &llm.api_key,
            llm.model.as_deref(),
            llm.small.as_deref(),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn write_row(
        conn: &Connection,
        mode: Mode,
        feature: &str,
        wire: &str,
        base_url: &str,
        api_key: &str,
        model: Option<&str>,
        small: Option<&str>,
    ) -> anyhow::Result<()> {
        conn.execute(
            "INSERT INTO credential (mode, feature, wire, base_url, api_key, model, small) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(mode, feature, wire) DO UPDATE SET
                 base_url = excluded.base_url, api_key = excluded.api_key,
                 model = excluded.model, small = excluded.small",
            params![mode_str(mode), feature, wire, base_url, api_key, model, small],
        )?;
        Ok(())
    }

    /// Read the singleton account row. Tokens are `Some` iff an expiry is stored;
    /// energy is `Some` iff a tier is stored — the two are written independently.
    fn read_account(conn: &Connection) -> anyhow::Result<(Option<Tokens>, Option<Energy>)> {
        let row = conn
            .query_row(
                "SELECT access_token, refresh_token, access_expires_at,
                        energy_remaining, energy_total, energy_resets_at, energy_tier
                 FROM account WHERE id = ?1",
                params![ACCOUNT_ID],
                |r| {
                    Ok((
                        r.get::<_, Option<String>>(0)?,
                        r.get::<_, Option<String>>(1)?,
                        r.get::<_, Option<String>>(2)?,
                        r.get::<_, Option<i64>>(3)?,
                        r.get::<_, Option<i64>>(4)?,
                        r.get::<_, Option<String>>(5)?,
                        r.get::<_, Option<String>>(6)?,
                    ))
                },
            )
            .optional()?;
        let Some((at, rt, exp, remaining, total, resets_at, tier)) = row else {
            return Ok((None, None));
        };
        let tokens = exp.map(|access_expires_at| Tokens {
            access_token: at.unwrap_or_default(),
            refresh_token: rt.unwrap_or_default(),
            access_expires_at,
        });
        let energy = tier.map(|tier| Energy {
            remaining: remaining.unwrap_or_default(),
            total: total.unwrap_or_default(),
            resets_at: resets_at.unwrap_or_default(),
            tier,
        });
        Ok((tokens, energy))
    }

    fn write_account(conn: &Connection, c: &Credentials) -> anyhow::Result<()> {
        let (at, rt, exp) = match &c.tokens {
            Some(t) => (Some(&t.access_token), Some(&t.refresh_token), Some(&t.access_expires_at)),
            None => (None, None, None),
        };
        let (remaining, total, resets_at, tier) = match &c.energy {
            Some(e) => (Some(e.remaining), Some(e.total), Some(&e.resets_at), Some(&e.tier)),
            None => (None, None, None, None),
        };
        conn.execute(
            "INSERT INTO account (id, access_token, refresh_token, access_expires_at,
                                  energy_remaining, energy_total, energy_resets_at, energy_tier)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT(id) DO UPDATE SET
                 access_token = excluded.access_token, refresh_token = excluded.refresh_token,
                 access_expires_at = excluded.access_expires_at,
                 energy_remaining = excluded.energy_remaining, energy_total = excluded.energy_total,
                 energy_resets_at = excluded.energy_resets_at, energy_tier = excluded.energy_tier",
            params![ACCOUNT_ID, at, rt, exp, remaining, total, resets_at, tier],
        )?;
        Ok(())
    }

    /// The pre-SQLite JSON store, imported once. Named for its legacy filename.
    const LEGACY_JSON: &str = "credentials.json";

    /// Import a legacy `credentials.json` into a never-written DB, then rename it to
    /// `.bak` so the import runs at most once. A malformed legacy file logs and is
    /// skipped (the user re-saves from Settings) rather than blocking boot.
    fn maybe_import_legacy(conn: &Connection, data_dir: &Path) -> anyhow::Result<()> {
        let legacy = data_dir.join(LEGACY_JSON);
        if !legacy.exists() {
            return Ok(());
        }
        let already_written: bool =
            conn.query_row("SELECT EXISTS(SELECT 1 FROM app_settings)", [], |r| r.get(0))?;
        if already_written {
            return Ok(());
        }
        let bytes = std::fs::read(&legacy).with_context(|| format!("reading {}", legacy.display()))?;
        match serde_json::from_slice::<Credentials>(&bytes) {
            Ok(c) => {
                let tx = conn.unchecked_transaction()?;
                write_all(&tx, &c)?;
                tx.commit()?;
                let bak = data_dir.join(format!("{LEGACY_JSON}.bak"));
                let _ = std::fs::rename(&legacy, &bak);
                tracing::info!(backup = %bak.display(), "imported legacy credentials.json into config.db");
            }
            Err(e) => tracing::warn!(error = %e, "legacy credentials.json unreadable; skipping import"),
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_mode_is_case_insensitive() {
        assert_eq!(parse_mode("byok"), Some(Mode::Byok));
        assert_eq!(parse_mode("XIAOYUANZHU"), Some(Mode::Xiaoyuanzhu));
        // Legacy values fold into xiaoyuanzhu.
        assert_eq!(parse_mode("FREE"), Some(Mode::Xiaoyuanzhu));
        assert_eq!(parse_mode(" login "), Some(Mode::Xiaoyuanzhu));
        assert_eq!(parse_mode("nope"), None);
    }

    #[test]
    fn legacy_mode_values_deserialize_to_xiaoyuanzhu() {
        // An older credentials.json with `"mode": "free"` (or "login") must still
        // load — the serde aliases fold it into xiaoyuanzhu, not a parse failure.
        for legacy in [r#"{"mode":"free"}"#, r#"{"mode":"login"}"#] {
            let c: Credentials = serde_json::from_str(legacy).unwrap();
            assert_eq!(c.mode, Mode::Xiaoyuanzhu);
        }
    }

    #[test]
    fn missing_file_is_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let c = Credentials::load(dir.path());
        assert_eq!(c.mode, Mode::Xiaoyuanzhu);
        assert!(c.llm.api_key.is_empty());
        assert!(c.tokens.is_none());
    }

    #[test]
    fn round_trips_through_disk() {
        let dir = tempfile::tempdir().unwrap();
        let c = Credentials {
            mode: Mode::Xiaoyuanzhu,
            device_id: "dev-1".into(),
            tokens: Some(Tokens {
                access_token: "acc".into(),
                refresh_token: "ref".into(),
                access_expires_at: "2026-06-29T00:00:00Z".into(),
            }),
            managed: Some(Managed {
                llm: LlmCredentials {
                    base_url: "https://songguo.xiaoyuanzhu.com".into(),
                    api_key: "sg-secret".into(),
                    model: None,
                    ..Default::default()
                },
                stt: vec![VendorKey { api_key: "sg-secret".into(), ..Default::default() }],
                ..Default::default()
            }),
            energy: Some(Energy { remaining: 70, total: 100, resets_at: "x".into(), tier: "free".into() }),
            identity: Some(Identity {
                email: "iloahz@example.com".into(),
                name: "Li".into(),
                tier: "standard".into(),
            }),
            ..Default::default()
        };
        c.save(dir.path()).unwrap();
        let back = Credentials::load(dir.path());
        assert_eq!(back.device_id, "dev-1");
        assert_eq!(back.tokens.as_ref().unwrap().access_token, "acc");
        assert_eq!(back.managed.as_ref().unwrap().llm.base_url, "https://songguo.xiaoyuanzhu.com");
        assert_eq!(back.energy.as_ref().unwrap().remaining, 70);
        assert_eq!(back.identity.as_ref().unwrap().email, "iloahz@example.com");
        assert_eq!(back.identity.as_ref().unwrap().tier, "standard");
    }

    #[test]
    fn both_modes_configs_coexist_across_a_switch() {
        // The user's BYOK keys and the broker's managed bundle are stored side by
        // side; flipping the active mode must not lose the other mode's config.
        let dir = tempfile::tempdir().unwrap();
        let c = Credentials {
            mode: Mode::Byok,
            llm: LlmCredentials { api_key: "byok-llm".into(), ..Default::default() },
            managed: Some(Managed {
                llm: LlmCredentials { api_key: "managed-llm".into(), ..Default::default() },
                ..Default::default()
            }),
            ..Default::default()
        };
        c.save(dir.path()).unwrap();

        // Switch to xiaoyuanzhu and re-save (as the settings handler would).
        let mut back = Credentials::load(dir.path());
        assert_eq!(back.llm.api_key, "byok-llm"); // BYOK still there while in byok
        back.mode = Mode::Xiaoyuanzhu;
        back.save(dir.path()).unwrap();

        // Both configs survive the round-trip; effective() follows the active mode.
        let after = Credentials::load(dir.path());
        assert_eq!(after.mode, Mode::Xiaoyuanzhu);
        assert_eq!(after.llm.api_key, "byok-llm", "BYOK config must persist across a switch");
        assert_eq!(after.managed.as_ref().unwrap().llm.api_key, "managed-llm");
        assert_eq!(after.effective().unwrap().llm.api_key, "managed-llm");
    }

    #[test]
    fn app_settings_get_set_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(get_setting(dir.path(), "pulse"), None); // unset
        set_setting(dir.path(), "pulse", "120").unwrap();
        assert_eq!(get_setting(dir.path(), "pulse").as_deref(), Some("120"));
        // Blank reads back as absent (→ the caller's built-in default).
        set_setting(dir.path(), "pulse", "  ").unwrap();
        assert_eq!(get_setting(dir.path(), "pulse"), None);
        // Coexists with the credential mode row in the same table.
        Credentials { mode: Mode::Byok, ..Default::default() }.save(dir.path()).unwrap();
        set_setting(dir.path(), "effort", "high").unwrap();
        assert_eq!(Credentials::load(dir.path()).mode, Mode::Byok);
        assert_eq!(get_setting(dir.path(), "effort").as_deref(), Some("high"));
    }

    #[test]
    fn wire_selection_persists_through_disk() {
        // The per-feature wire id is stored and restored, so a future non-default
        // vendor choice survives a restart.
        let dir = tempfile::tempdir().unwrap();
        let c = Credentials {
            mode: Mode::Byok,
            tts: VendorKey { wire: "volcengine".into(), api_key: "k".into(), ..Default::default() },
            ..Default::default()
        };
        c.save(dir.path()).unwrap();
        let back = Credentials::load(dir.path());
        assert_eq!(back.tts.wire_opt(), Some("volcengine"));
        // An unset wire stays empty (→ the feature default at init time).
        assert_eq!(back.stt.wire_opt(), None);
    }

    #[test]
    fn imports_legacy_credentials_json_once() {
        let dir = tempfile::tempdir().unwrap();
        // A pre-SQLite store with a legacy mode value and a BYOK key.
        let legacy = r#"{"mode":"free","llm":{"base_url":"","api_key":"old-key","model":null}}"#;
        std::fs::write(dir.path().join("credentials.json"), legacy).unwrap();

        let c = Credentials::load(dir.path());
        assert_eq!(c.mode, Mode::Xiaoyuanzhu, "legacy free → xiaoyuanzhu");
        assert_eq!(c.llm.api_key, "old-key", "legacy key imported");
        // The JSON is renamed so the import can't run twice.
        assert!(!dir.path().join("credentials.json").exists());
        assert!(dir.path().join("credentials.json.bak").exists());

        // A second load reads purely from the DB (no re-import) and is unchanged.
        let again = Credentials::load(dir.path());
        assert_eq!(again.llm.api_key, "old-key");
    }

    /// The published model menu has to survive the store, and this is not academic:
    /// `broker::refresh` fetches it, saves, and returns — every later reader, including
    /// the capability init that builds the tool description from it, loads back from
    /// disk. Dropped on write, the agent is told "no menu is published" while the
    /// broker is publishing one.
    #[test]
    fn the_published_model_menu_survives_the_store() {
        let dir = tempfile::tempdir().unwrap();
        let mut c = Credentials::default();
        c.managed = Some(Managed {
            image: vec![VendorKey {
                api_key: "k".into(),
                model: Some("gpt-image-2".into()),
                models: vec![
                    ModelOffer { name: "gpt-image-2".into(), quality: 90, speed: 40, price: 30 },
                    ModelOffer { name: "gpt-image-1-mini".into(), quality: 50, speed: 90, price: 5 },
                ],
                ..Default::default()
            }],
            ..Default::default()
        });
        c.save(dir.path()).unwrap();

        let back = Credentials::load(dir.path());
        let image = &back.managed.as_ref().unwrap().image[0];
        assert_eq!(image.models.len(), 2, "the menu was dropped on the way through");
        assert_eq!(image.models[0].name, "gpt-image-2");
        assert_eq!(image.models[1].price, 5);

        // A capability the broker offered nothing for keeps no row at all, rather than
        // one blank provider standing in for "no provider".
        assert!(back.managed.as_ref().unwrap().stt.is_empty());
    }

    /// A capability served over several wires keeps every one of them, in rank order,
    /// across the store. The old key was `(mode, feature)`, so the second wire did not
    /// merely lose its rank — it overwrote the first.
    #[test]
    fn every_wire_of_one_capability_survives_the_store() {
        let dir = tempfile::tempdir().unwrap();
        let mut c = Credentials::default();
        c.managed = Some(Managed {
            image: vec![
                VendorKey {
                    wire: "openai-responses".into(),
                    base_url: "https://songguo.example/v1/responses".into(),
                    api_key: "k".into(),
                    model: Some("gpt-image-2".into()),
                    models: vec![ModelOffer {
                        name: "gpt-image-2".into(),
                        quality: 96,
                        speed: 45,
                        price: 90,
                    }],
                    carrier: "gpt-5.4".into(),
                },
                VendorKey {
                    wire: "openai-images".into(),
                    base_url: "https://songguo.example/v1/images/generations".into(),
                    api_key: "k".into(),
                    model: Some("doubao-seedream-5.0-lite".into()),
                    models: vec![ModelOffer {
                        name: "doubao-seedream-5.0-lite".into(),
                        quality: 75,
                        speed: 80,
                        price: 40,
                    }],
                    carrier: String::new(),
                },
            ],
            ..Default::default()
        });
        c.save(dir.path()).unwrap();

        let back = Credentials::load(dir.path());
        let image = &back.managed.as_ref().unwrap().image;
        assert_eq!(image.len(), 2, "the second wire overwrote the first");
        assert_eq!(image[0].wire, "openai-responses", "rank must survive the round-trip");
        assert_eq!(image[1].wire, "openai-images");
        assert_eq!(image[1].models[0].name, "doubao-seedream-5.0-lite");
        // The carrier rides with its wire, and only with the wire that has one.
        assert_eq!(image[0].carrier_opt(), Some("gpt-5.4"));
        assert_eq!(image[1].carrier_opt(), None);

        // A later refresh that drops a wire must drop its row too, not leave it behind
        // as a provider the broker has stopped offering.
        let mut fewer = back;
        fewer.managed.as_mut().unwrap().image.truncate(1);
        fewer.save(dir.path()).unwrap();
        let after = Credentials::load(dir.path());
        assert_eq!(after.managed.as_ref().unwrap().image.len(), 1);
        assert_eq!(after.managed.as_ref().unwrap().image[0].wire, "openai-responses");
    }

    /// **The upgrade path, on a store that already exists.** Every other test here
    /// builds a fresh DB, which is created with the current key and so never exercises
    /// the rebuild. An installed agent has the old `(mode, feature)` key, and getting
    /// this wrong empties the credential store on first launch after an update.
    #[test]
    fn an_existing_store_is_rekeyed_without_losing_a_row() {
        let dir = tempfile::tempdir().unwrap();
        {
            let conn = rusqlite::Connection::open(path(dir.path())).unwrap();
            conn.execute_batch(
                "CREATE TABLE credential (
                     mode     TEXT NOT NULL,
                     feature  TEXT NOT NULL,
                     wire     TEXT NOT NULL DEFAULT '',
                     base_url TEXT NOT NULL DEFAULT '',
                     api_key  TEXT NOT NULL DEFAULT '',
                     model    TEXT,
                     small    TEXT,
                     models   TEXT,
                     PRIMARY KEY (mode, feature)
                 );
                 INSERT INTO credential (mode, feature, wire, base_url, api_key, model, models)
                 VALUES ('xiaoyuanzhu', 'image', 'openai-images',
                         'https://songguo.example/v1/images/generations', 'old-key',
                         'doubao-seedream-5.0-lite',
                         '[{\"name\":\"doubao-seedream-5.0-lite\",\"quality\":75,\"speed\":80,\"price\":40}]');",
            )
            .unwrap();
        }

        let loaded = Credentials::load(dir.path());
        let image = &loaded.managed.as_ref().unwrap().image;
        assert_eq!(image.len(), 1, "the stored row did not survive the rebuild");
        assert_eq!(image[0].api_key, "old-key");
        assert_eq!(image[0].models[0].name, "doubao-seedream-5.0-lite");

        // And the point of the rebuild: a second wire can now sit beside the first
        // instead of replacing it.
        let mut grown = loaded;
        grown.managed.as_mut().unwrap().image.push(VendorKey {
            wire: "openai-responses".into(),
            api_key: "k".into(),
            ..Default::default()
        });
        grown.save(dir.path()).unwrap();
        assert_eq!(Credentials::load(dir.path()).managed.unwrap().image.len(), 2);
    }

    /// A `credentials.json` written before the slots became lists still loads — serde
    /// fails the whole `Credentials` on one bad field, so a single-object slot would
    /// take the account and the BYOK keys down with it.
    #[test]
    fn a_legacy_single_wire_slot_deserializes_as_a_list() {
        let legacy = r#"{
            "mode":"xiaoyuanzhu",
            "managed":{
                "llm":{"base_url":"https://songguo.example/v1","api_key":"k"},
                "image":{"api_key":"ik","model":"gpt-image-2"},
                "stt":{"api_key":"sk"}
            }
        }"#;
        let c: Credentials = serde_json::from_str(legacy).unwrap();
        let m = c.managed.unwrap();
        assert_eq!(m.image.len(), 1);
        assert_eq!(m.image[0].model.as_deref(), Some("gpt-image-2"));
        assert_eq!(m.stt[0].api_key, "sk");
        assert!(m.video.is_empty(), "an absent slot is an empty list, not a blank provider");
    }

    #[test]
    fn effective_picks_byok_or_managed() {
        let mut c = Credentials::default();
        assert_eq!(c.mode, Mode::Xiaoyuanzhu); // xiaoyuanzhu is the default

        c.mode = Mode::Byok;
        c.llm.api_key = "byok-key".into();
        assert_eq!(c.effective().unwrap().llm.api_key, "byok-key");

        // xiaoyuanzhu with no configs → nothing in effect (callers fall back to env).
        c.mode = Mode::Xiaoyuanzhu;
        assert!(c.effective().is_none());

        c.managed = Some(Managed {
            llm: LlmCredentials {
                base_url: "https://songguo.xiaoyuanzhu.com".into(),
                api_key: "managed-key".into(),
                model: None,
                ..Default::default()
            },
            stt: vec![VendorKey { api_key: "managed-stt".into(), ..Default::default() }],
            ..Default::default()
        });
        let e = c.effective().unwrap();
        assert_eq!(e.llm.api_key, "managed-key");
        assert_eq!(e.stt[0].key_opt(), Some("managed-stt"));
        assert_ne!(e.llm.api_key, "byok-key"); // BYOK ignored while managed is live
    }

    #[test]
    fn debug_redacts_secrets() {
        let c = Credentials {
            llm: LlmCredentials { base_url: "https://x".into(), api_key: "sk-super-secret".into(), model: None, ..Default::default() },
            vision: VendorKey { api_key: "vision-super-secret".into(), ..Default::default() },
            tokens: Some(Tokens {
                access_token: "access-super-secret".into(),
                refresh_token: "refresh-super-secret".into(),
                access_expires_at: "x".into(),
            }),
            ..Default::default()
        };
        let rendered = format!("{c:?}");
        for leak in ["sk-super-secret", "vision-super-secret", "access-super-secret", "refresh-super-secret"] {
            assert!(!rendered.contains(leak), "leaked {leak}: {rendered}");
        }
        assert!(rendered.contains("<redacted>"));
    }

    #[cfg(unix)]
    #[test]
    fn saved_file_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        Credentials::default().save(dir.path()).unwrap();
        let mode = std::fs::metadata(path(dir.path())).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }
}
