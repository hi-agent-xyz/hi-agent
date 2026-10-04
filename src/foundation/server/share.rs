//! A share: one view or one attachment, published as a page somebody with no credential can
//! open — and the check a view passes before it may be one. See
//! [`docs/arch/sharing.md`](../../../docs/arch/sharing.md).
//!
//! **A view is checked; an attachment is not.** A shared view is served to somebody with no
//! session, so it is rendered here the way that person will see it — **with `/api/*` refused**,
//! with view-owned resources excepted — and the run is read for answers that would otherwise each
//! need a mechanism of their own:
//!
//! | Question | Read from |
//! |---|---|
//! | may this be shared at all? | the page's own verdict, plus a blank frame |
//! | what is the page's HTML? | the settled DOM |
//!
//! The first is the one that earns the check. A view that fetches its own data renders
//! half-empty under a share's scope and **looks fine in its source**; without this, the owner
//! is not the one who finds out, the person they sent it to is.
//!
//! An attachment fetches nothing, so there is nothing that could render half-empty: its page is
//! the host's own viewer, written here from the render page when it is asked for, with the
//! thing itself in the HTML for whatever reads it without running anything.

use crate::body::capabilities::view_render;
use crate::foundation::attachments;

/// What one request the render made was for.
#[derive(Debug, PartialEq, Eq)]
enum Asked {
    /// Something already in the page: a `data:` or `blob:` URL. It reaches nothing, and every
    /// `<video controls>` makes half a dozen of them — the player's own icons are `data:` SVGs.
    Inert,
    /// A path on this core, without its query: what a scope is written in, and what a person
    /// reading a refusal wants to see.
    Here(String),
    /// Another origin. **A shared page cannot depend on one** — the view builder's own rule is
    /// never to hotlink — and reading such a URL by its path alone would let
    /// `https://elsewhere.example/views/mine/x.jpg` pass as this view's own file.
    Elsewhere(String),
}

/// Read one requested URL against the origin the render loaded the page from.
fn asked(url: &str, origin: &str) -> Asked {
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return Asked::Inert;
    }
    match url.strip_prefix(origin.trim_end_matches('/')).filter(|rest| rest.starts_with('/')) {
        // The query is dropped: share grants are written in terms of paths.
        Some(path) => Asked::Here(path.split(['?', '#']).next().unwrap_or(path).to_string()),
        None => Asked::Elsewhere(url.chars().take(120).collect()),
    }
}

/// The attachment a path on this core names — `/api/attachments/<id>` and everything under
/// it: the preview, the description, the playable route and the copy it points at.
fn attachment_in(path: &str) -> Option<&str> {
    let rest = path.strip_prefix("/api/attachments/")?;
    attachments::parse_id(rest.split('/').next()?)
}

/// Whether a URL the page asked for is one of a view's own files.
///
/// The list is derived from the ref rather than stored, so it cannot drift from what
/// the view is: this view's own compiled module, its `<ref>.assets/` folder, and the
/// build's shared assets. **Never `/views/` at large** — that is one wildcard route
/// with every view's source and every build artifact behind it, and opening it would
/// hand over every view the agent has built to share one poster.
fn in_scope(path: &str, view_ref: &str, module_url: &str) -> bool {
    if path == module_url {
        return true;
    }
    if path.starts_with("/assets/") {
        return true;
    }
    // This view's own picture. It is what `og:image` points at, and a link preview is
    // scraped by something holding no cookie — so leaving it out meant every preview
    // 401'd while the page itself opened fine, which is the failure that looks like the
    // chat app being broken rather than like us.
    if path == format!("/views/_shots/ref/{view_ref}.png") {
        return true;
    }
    // A ref owns only its explicitly named resource directory, at any nesting depth.
    // A project's directory may also contain other views, manifests and private data.
    path.starts_with(&format!("/views/{view_ref}.assets/"))
        && !path.starts_with(&format!("/views/{view_ref}.assets/_resources/"))
}

/// What one check found.
#[derive(Debug, Clone)]
pub struct ShareCheck {
    /// The view rendered, and everything it asked for was in scope. Only a `true`
    /// here may become a share record.
    pub ok: bool,
    /// Why not, in the order found — empty when `ok`. Written to be read by the
    /// person deciding whether to share, so each line names the thing to fix.
    pub refusals: Vec<String>,
    /// The settled DOM: what an agent reading the URL gets, and what a link preview
    /// scrapes. `None` only when the render itself failed.
    pub html: Option<String>,
    /// Legacy field retained for callers of the render check; grants never use requests.
    pub attachments: Vec<String>,
    /// The picture, for the owner to look at before publishing. The check renders it
    /// anyway, and *"this is what you are about to publish"* is worth more than a
    /// list of paths.
    pub png: Vec<u8>,
}

/// Render `module_url` as a visitor would see it and judge whether it may be shared.
///
/// `view_ref` is the name the share will carry; it decides the scope, so it is read
/// here and not taken on trust from the caller's intent.
pub async fn check(view_ref: &str, module_url: &str) -> anyhow::Result<ShareCheck> {
    let ctx = crate::mind::views::render_context()
        .ok_or_else(|| anyhow::anyhow!("no render context: this core cannot render a view yet"))?;

    let rendered =
        view_render::render(&view_render::RenderRequest::for_share(&ctx.base_url, module_url))
            .await?;

    let mut refusals = Vec::new();

    // The page's own account first, because it names the cause. A blank frame with no
    // errors is the case this whole check exists for: it looks like a pass everywhere
    // except on the screen.
    if rendered.failed {
        refusals.push("the view did not mount".to_string());
    }
    if rendered.timed_out {
        refusals.push("the view never finished rendering".to_string());
    }
    if rendered.blank {
        refusals.push("the view rendered blank".to_string());
    }
    refusals.extend(rendered.problems.iter().cloned());

    // Then what it reached for. A refused request is not a defect in the view — it is
    // a view that cannot be shared *as it is*, which is a different sentence and the
    // one the owner needs.
    for url in &rendered.requested {
        let path = match asked(url, &ctx.base_url) {
            Asked::Inert => continue,
            Asked::Here(path) => path,
            Asked::Elsewhere(url) => {
                let refusal = format!(
                    "it asks for {url}, which is on somebody else's machine — a shared page \
                     carries its own files"
                );
                if !refusals.contains(&refusal) {
                    refusals.push(refusal);
                }
                continue;
            }
        };
        if path == "/render/view" {
            continue; // the host page itself
        }
        // A global attachment ID has no view ownership. The view must use its
        // registered resource path, which also works for delayed requests.
        if let Some(id) = attachment_in(&path) {
            refusals.push(format!("attachment att:{id} is global; register a view resource and use its view path"));
            continue;
        }
        // The route itself rejects unknown names; publication separately validates
        // all bindings before this render becomes a share.
        if path.starts_with(&format!("/views/{view_ref}.assets/_resources/"))
            || in_scope(&path, view_ref, module_url)
        {
            continue;
        }
        let refusal = if path.starts_with("/api/") {
            format!("it reads {path} while it renders, and a shared view is never given the API")
        } else {
            format!("it asks for {path}, which is outside this view's own files")
        };
        if !refusals.contains(&refusal) {
            refusals.push(refusal);
        }
    }

    Ok(ShareCheck {
        ok: refusals.is_empty(),
        refusals,
        html: rendered.html,
        attachments: Vec::new(),
        png: rendered.png,
    })
}

/// `app_settings` key holding the shares, as a JSON array.
///
/// **In the config store, not the views tree**, for the reason bookmarks are: the tree
/// is disposable and `factory/` is re-seeded on every boot, and a share is a decision
/// the person made about publishing something. It has to outlive an upgrade.
const SHARES_KEY: &str = "view_shares";

/// Where a shared view's rendered page lives — a tool dir inside the views tree, beside
/// `_compiled/` and `_shots/`, and disposable in the same way.
///
/// The page is the *check's output*, so losing it means the view has to be checked
/// again before it can be served — which is the correct consequence, not a bug to
/// paper over with a fallback that serves a mount point with no content in it. An
/// attachment's page is written when it is asked for, so it has none.
fn shares_dir(data_dir: &std::path::Path) -> std::path::PathBuf {
    data_dir.join("views").join("_shares")
}

fn page_path(data_dir: &std::path::Path, view_ref: &str) -> std::path::PathBuf {
    shares_dir(data_dir).join(format!("{view_ref}.html"))
}

/// The first path segment a shared attachment is at: `/att/<id>`. Reserved, so no view
/// can be shared under it.
const ATTACHMENT_SEGMENT: &str = "att";

/// What a share publishes, as the one name either is known by everywhere
/// (`docs/arch/showing.md` § *The model*).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Shared {
    View(String),
    Attachment(String),
}

impl Shared {
    /// Read what a caller named: an `att:` id, or a view's ref with or without its `view:`.
    pub fn parse(named: &str) -> Result<Self, String> {
        let named = named.trim();
        if named.starts_with(attachments::PREFIX) {
            return attachments::ref_id(named)
                .map(|id| Self::Attachment(id.to_string()))
                .ok_or_else(|| format!("`{named}` is not an attachment id"));
        }
        let view_ref = named.strip_prefix("view:").unwrap_or(named).trim();
        if !crate::mind::views::valid_ref(view_ref) {
            return Err("only a named view, or an attachment, can be shared".to_string());
        }
        Ok(Self::View(view_ref.to_string()))
    }

    fn of(&self) -> String {
        match self {
            Self::View(r) => format!("view:{r}"),
            Self::Attachment(id) => format!("{}{id}", attachments::PREFIX),
        }
    }

    /// The path it is shared at, without its leading `/`: a view at its ref, an attachment
    /// at `att/<id>`.
    pub fn name(&self) -> String {
        match self {
            Self::View(r) => r.clone(),
            Self::Attachment(id) => format!("{ATTACHMENT_SEGMENT}/{id}"),
        }
    }
}

/// One published view or attachment.
///
/// The key is stored as a hash for the same reason a credential is: it is a bearer
/// token, it exists in plaintext exactly once, and what is kept here is only enough to
/// recognise it. `None` is a public share.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Share {
    /// What is published: `view:<ref>` or `att:<id>`.
    pub of: String,
    /// The module this share's page mounts: a view's compiled module, or the stage module
    /// the host writes for an attachment. Kept because the scope is enforced on every
    /// request and is written in terms of it — re-compiling to find out would also mean a
    /// recompile could quietly widen what a share serves.
    pub module_url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key_hash: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    /// What this is, in a sentence — the `<meta name="description">` and the
    /// `og:description`. Written by the agent at share time, because it is the half of
    /// the page a reader sees before deciding to open it.
    #[serde(default)]
    pub description: String,
    /// The attachments this share serves, by id: the one it is of, or those fixed at
    /// publication from earlier share records. New views use named resources.
    #[serde(default)]
    pub attachments: Vec<String>,
    /// Names authorized at publication; each name resolves to its current attachment.
    #[serde(default)]
    pub resources: Vec<String>,
}

impl Share {
    /// What it publishes, or `None` for a record this build cannot read.
    pub fn shared(&self) -> Option<Shared> {
        Shared::parse(&self.of).ok()
    }

    /// The path it is at, without its leading `/`.
    pub fn name(&self) -> Option<String> {
        self.shared().map(|s| s.name())
    }

    /// Whether `presented` opens this share. A public share is opened by anybody.
    pub fn opened_by(&self, presented: Option<&str>) -> bool {
        match &self.key_hash {
            None => true,
            Some(want) => presented.is_some_and(|key| &hex_hash(key) == want),
        }
    }

    /// Whether this share serves `path`: its own page, and exactly what that page needs.
    fn serves(&self, path: &str) -> bool {
        let Some(shared) = self.shared() else { return false };
        if path.trim_end_matches('/') == format!("/{}", shared.name()) {
            return true;
        }
        if attachment_in(path).is_some_and(|id| self.attachments.iter().any(|a| a == id)) {
            return true;
        }
        match &shared {
            Shared::View(view_ref) => {
                let prefix = format!("/views/{view_ref}.assets/_resources/");
                if let Some(rest) = path.strip_prefix(&prefix) {
                    return rest.split_once('/').is_some_and(|(name, spec)| {
                        self.resources.iter().any(|allowed| allowed == name)
                            && matches!(spec, "about.v1" | "preview.v1" | "original" | "playable" | "proxy.v1")
                    });
                }
                in_scope(path, view_ref, &self.module_url)
            }
            Shared::Attachment(_) => path == self.module_url || path.starts_with("/assets/"),
        }
    }
}

/// SHA-256, hex — the same choice, and the same reason, as a surface credential:
/// a 32-byte random token is not guessable, so a slow KDF would buy nothing.
fn hex_hash(token: &str) -> String {
    crate::foundation::surfaces::token_hash(token).iter().map(|b| format!("{b:02x}")).collect()
}

/// Every share this core is publishing. A store that cannot be read reads as none,
/// which fails closed: nothing is served rather than everything.
pub fn read_shares(data_dir: &std::path::Path) -> Vec<Share> {
    crate::foundation::credentials::get_setting(data_dir, SHARES_KEY)
        .and_then(|raw| serde_json::from_str::<Vec<Share>>(&raw).ok())
        .unwrap_or_default()
}

fn write_shares(data_dir: &std::path::Path, shares: &[Share]) -> anyhow::Result<()> {
    let encoded = serde_json::to_string(shares)?;
    crate::foundation::credentials::set_setting(data_dir, SHARES_KEY, &encoded)
}

/// The share published at `/<name>`, if any.
pub fn find(data_dir: &std::path::Path, name: &str) -> Option<Share> {
    read_shares(data_dir).into_iter().find(|s| s.name().as_deref() == Some(name))
}

/// Which of the [two kinds of share](../../../docs/arch/sharing.md#two-kinds-of-share) a
/// record is. Serialized as the word the owner's UI puts on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Public,
    Unlisted,
}

impl Share {
    pub fn kind(&self) -> Kind {
        if self.key_hash.is_some() { Kind::Unlisted } else { Kind::Public }
    }
}

/// Who can reach a link: anybody it is handed to, or only something on this machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Reach {
    /// The core serves a claimed name, and the link is under it.
    Anywhere,
    /// The core has no name yet: the link is a path on this machine's own address.
    ThisMachine,
}

/// The link to hand out for a share at `path`, and who it reaches.
///
/// **One rule, for every place that hands a link to somebody** — `hi_share` telling the
/// agent and `POST /api/shares` telling the owner's window. The claimed name when there
/// is one; otherwise the bare path, never a `localhost` URL somebody would send to a
/// friend. The key rides the path either way: it exists in plaintext exactly once, and a
/// core with no name would otherwise publish something nobody could ever open.
pub async fn link(data_dir: &std::path::Path, path: &str, key: Option<&str>) -> (String, Reach) {
    let base = super::surfaces::named_base_url(data_dir).await;
    let keyed = match key {
        Some(key) => format!("{path}?key={key}"),
        None => path.to_string(),
    };
    match base {
        Some(base) => (format!("{base}{keyed}"), Reach::Anywhere),
        None => (keyed, Reach::ThisMachine),
    }
}

/// First path segments this core serves itself, and which a share therefore cannot
/// take. The username-versus-route trap for the third time in this system — handles
/// against community routes, handles against DNS, and now a view's name against the
/// core's own paths.
///
/// Deliberately broader than today's route table. A share *can* be released, unlike a
/// handle, so this need not be as absolute as the registry's list — but a link already
/// sent cannot be un-sent, which is why it is checked when the share is made and not
/// when it is served.
///
/// `att` is where shared attachments are.
const RESERVED: &[&str] = &[
    "api", "views", "assets", "generated", "up", "render", "auth", "account", "inspect",
    "healthz", "mcp", "favicon.ico", "robots.txt", "index.html", "static", "well-known",
    ".well-known", "sw.js", "manifest.json", ATTACHMENT_SEGMENT,
];

/// Why `view_ref` cannot be a share's address, or `None` if it can.
pub fn name_collision(view_ref: &str) -> Option<String> {
    let first = view_ref.split('/').next().unwrap_or_default();
    RESERVED
        .contains(&first)
        .then(|| format!("`{first}` is a path this agent serves itself, so a view cannot be shared under it"))
}

/// What opening a share produced. The key exists here and nowhere else.
#[derive(Debug, Clone)]
pub struct Opened {
    pub path: String,
    /// The plaintext key, for an unlisted share. Shown once.
    pub key: Option<String>,
}

/// Why something could not be shared.
#[derive(Debug, Clone)]
pub enum Refused {
    /// It is not a thing that can be shared under that name: a name that collides with
    /// something this core serves, a system view, or not a name at all.
    Name(String),
    /// The id names no attachment this core holds.
    Unknown(String),
    /// The check ran and the view did not pass it.
    Check(Vec<String>),
    /// The check could not run at all.
    Broken(String),
}

/// Publish a view — once it has passed the check — or an attachment.
///
/// **The check is the only path to a view's share record** — there is no argument for
/// publishing something nobody has looked at, and this is the function that makes that
/// structural rather than a rule somebody has to remember.
pub async fn open(
    data_dir: &std::path::Path,
    named: &str,
    unlisted: bool,
    description: &str,
) -> Result<Opened, Refused> {
    let shared = Shared::parse(named).map_err(Refused::Name)?;
    let mut share = match &shared {
        Shared::Attachment(id) => {
            if attachments::probe(data_dir, id).await.is_none() {
                return Err(Refused::Unknown(format!("att:{id} is not an attachment this agent holds")));
            }
            Share {
                of: shared.of(),
                module_url: attachments::stage_module_url(id),
                key_hash: None,
                created_at: chrono::Utc::now(),
                description: description.trim().to_string(),
                attachments: vec![id.clone()],
                resources: Vec::new(),
            }
        }
        Shared::View(view_ref) => {
            let (module_url, checked, resources) = check_view(data_dir, view_ref).await?;
            Share {
                of: shared.of(),
                module_url,
                key_hash: None,
                created_at: chrono::Utc::now(),
                description: description.trim().to_string(),
                attachments: checked.attachments,
                resources,
            }
        }
    };

    let key = unlisted.then(crate::foundation::surfaces::random_token);
    share.key_hash = key.as_deref().map(hex_hash);
    let name = shared.name();
    let mut shares = read_shares(data_dir);
    shares.retain(|s| s.name().as_deref() != Some(name.as_str()));
    shares.push(share);
    write_shares(data_dir, &shares).map_err(|e| Refused::Broken(format!("{e:#}")))?;

    Ok(Opened { path: format!("/{name}"), key })
}

/// Compile, check and write the page of the view at `view_ref`: the module it publishes, and
/// what the check found.
async fn check_view(
    data_dir: &std::path::Path,
    view_ref: &str,
) -> Result<(String, ShareCheck, Vec<String>), Refused> {
    if let Some(why) = name_collision(view_ref) {
        return Err(Refused::Name(why));
    }
    // `factory/*` is this core's own dashboard over its own API, so sharing one is a
    // category error rather than a view that happens to fail. The check would refuse them
    // all anyway; saying so directly saves a render and gives a better reason.
    if view_ref.starts_with(crate::mind::views::factory::PREFIX) {
        return Err(Refused::Name(
            "a system view is this agent's own dashboard over its own API, so it is not a thing to publish".to_string(),
        ));
    }
    // Compiled here rather than taken from the caller: the thing checked has to be the
    // thing published, and a module handed in could be neither.
    let source = crate::mind::views::resolve_ref(data_dir, view_ref)
        .await
        .map_err(|e| Refused::Broken(e))?;
    let ctx = crate::mind::views::render_context()
        .ok_or_else(|| Refused::Broken("this core cannot render a view yet".to_string()))?;
    let module_url = ctx
        .compiler
        .compile(&source)
        .await
        .map_err(|e| Refused::Broken(format!("the view did not compile: {e}")))?;
    let resources = super::view_resources::bindings(data_dir, view_ref)
        .await.map_err(|e| Refused::Check(vec![e]))?
        .into_keys().collect();

    let checked =
        check(view_ref, &module_url).await.map_err(|e| Refused::Broken(format!("{e:#}")))?;
    if !checked.ok {
        return Err(Refused::Check(checked.refusals));
    }
    let Some(html) = checked.html.as_deref() else {
        return Err(Refused::Broken("the render returned no page".to_string()));
    };

    let path = page_path(data_dir, view_ref);
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| Refused::Broken(format!("making {}: {e}", parent.display())))?;
    }
    tokio::fs::write(&path, html)
        .await
        .map_err(|e| Refused::Broken(format!("writing {}: {e}", path.display())))?;

    // **A link with no picture is half a link.** `og:image` is this view's own shot, and a view
    // that was only ever reviewed has none — so a page published to be pasted into a chat
    // arrived there as a bare line of text. Taken here, once, and only when there is none.
    // It is a second render today, on the second browser this publish launches; the one queue
    // phase 4 puts in front of them is what makes it free.
    if super::view_shots::url_for_ref(data_dir, view_ref).is_none() {
        super::view_shots::take_ref(data_dir, view_ref, &module_url).await;
    }
    Ok((module_url, checked, resources))
}

/// Stop publishing what `named` names. Idempotent, and it takes a view's page with it — a
/// page left behind is a file that still answers to anyone who reaches it directly.
///
/// **The link keeps working until the edge forgets it**, which is a property of serving
/// uncredentialed pages and not something this can fix. Say so where a person can read
/// it rather than implying otherwise.
pub async fn close(data_dir: &std::path::Path, named: &str) -> anyhow::Result<()> {
    let shared = Shared::parse(named).map_err(|why| anyhow::anyhow!(why))?;
    let name = shared.name();
    let mut shares = read_shares(data_dir);
    shares.retain(|s| s.name().as_deref() != Some(name.as_str()));
    write_shares(data_dir, &shares)?;
    if let Shared::View(view_ref) = &shared {
        let _ = tokio::fs::remove_file(page_path(data_dir, view_ref)).await;
    }
    Ok(())
}

/// The cookie an unlisted share's key is carried in once the page has been opened.
///
/// **The page's sub-resources are why this exists.** A browser asks for the module and
/// the pictures without the query string, so a key that only ever lived in the URL
/// would open the page and nothing on it. Same seam as `POST /api/session`, different
/// scope: a session says *this surface may reach me*, this says *this caller may read
/// these paths*.
pub const SHARE_COOKIE: &str = "hi_share";

/// The key a request is presenting, from the query string or the cookie.
fn presented_key(uri: &axum::http::Uri, headers: &axum::http::HeaderMap) -> Option<String> {
    let from_query = uri.query().and_then(|q| {
        q.split('&').find_map(|pair| pair.strip_prefix("key=").map(|v| v.to_string()))
    });
    from_query.or_else(|| {
        headers.get_all(axum::http::header::COOKIE).iter().find_map(|h| {
            h.to_str().ok()?.split(';').find_map(|pair| {
                pair.trim().strip_prefix(SHARE_COOKIE)?.strip_prefix('=').map(str::to_string)
            })
        })
    })
}

/// Whether an **uncredentialed** request may have this path, because a share grants it.
///
/// Read by the gate, so it answers for the page *and* everything the page pulls: a
/// shared view whose pictures 401 is not shared. The scope is still the derived one —
/// the page's module, its own resource folder and registered names, `/assets/*`, and
/// attachments retained by legacy share records.
pub fn grants(
    data_dir: &std::path::Path,
    uri: &axum::http::Uri,
    headers: &axum::http::HeaderMap,
) -> bool {
    let path = uri.path();
    let key = presented_key(uri, headers);
    read_shares(data_dir)
        .iter()
        .any(|share| share.opened_by(key.as_deref()) && share.serves(path))
}

/// `<`, `&` and `"` in a value about to sit inside an HTML attribute.
fn escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// The `<meta>` a render page reads the module it mounts from when its URL carries none —
/// which a shared page's never does: it is at `/<name>`, not at `/render/view?module=…`
/// (`appearance/web/src/render/main.tsx`). Without it the live half mounted nothing over
/// the static half, and the page went blank the moment its script ran.
const MODULE_META: &str = "hi-view-module";

/// The page a visitor gets: `html` — the checked DOM of a view, or the render page an
/// attachment is drawn on — plus the tags that make it legible to something that is not a
/// browser, and the module its live half mounts.
///
/// A view's captured document is the render page's, and that is the right page rather than a
/// convenient one — it mounts exactly one view and deliberately does **not** mount
/// `SessionProvider`, so it opens no microphone, no camera and no channel long-poll and
/// registers as nobody. That is what a shared page has to be anyway.
///
/// `picture` is absolute: a link preview is fetched by a server that did not load this page,
/// and a relative `og:image` is resolved against nothing.
fn page_with(share: &Share, label: &str, html: &str, picture: Option<&str>) -> String {
    let title = if share.description.is_empty() { label } else { share.description.as_str() };
    let mut tags = format!(
        "<meta name=\"description\" content=\"{d}\">\n\
         <meta property=\"og:type\" content=\"website\">\n\
         <meta property=\"og:title\" content=\"{t}\">\n\
         <meta property=\"og:description\" content=\"{d}\">\n\
         <meta name=\"{MODULE_META}\" content=\"{m}\">\n",
        d = escape(&share.description),
        t = escape(label),
        m = escape(&share.module_url),
    );
    if let Some(picture) = picture {
        tags.push_str(&format!("<meta property=\"og:image\" content=\"{}\">\n", escape(picture)));
    }
    let Some(at) = html.find("</head>") else {
        // No head to inject into means the capture is not a document we recognise.
        // Serve it anyway — the content is the point, and the tags are the garnish.
        return html.to_string();
    };
    let page = format!("{}{tags}{}", &html[..at], &html[at..]);
    // The render page's own title says what it is to the harness, not to a reader.
    match (page.find("<title>"), page.find("</title>")) {
        (Some(open), Some(close)) if open < close => format!(
            "{}<title>{}</title>{}",
            &page[..open],
            escape(title),
            &page[close + "</title>".len()..]
        ),
        _ => page,
    }
}

/// The page an attachment is shared as: the render page with the host's own viewer mounted
/// on it, and the thing itself already in the body — a picture as an `<img>`, a clip as a
/// `<video>` — so an agent or a link preview reading the HTML gets the content without
/// running anything, and a browser gets the viewer over it.
fn attachment_page(
    share: &Share,
    id: &str,
    probe: &attachments::Probe,
    render_page: &str,
    picture: &str,
) -> String {
    let label = probe.what();
    let alt = escape(if share.description.is_empty() { &label } else { &share.description });
    let fit = "display:block;max-width:100%;max-height:100vh;margin:0 auto;object-fit:contain";
    let body = match probe.kind {
        attachments::Kind::Clip => format!(
            "<video src=\"{}\" poster=\"{}\" controls playsinline preload=\"metadata\" style=\"{fit}\" aria-label=\"{alt}\"></video>",
            attachments::playable_url(id),
            attachments::preview_url(id),
        ),
        attachments::Kind::Picture => {
            format!("<img src=\"{}\" alt=\"{alt}\" style=\"{fit}\">", attachments::url(id))
        }
    };
    let page = render_page.replacen(
        "<div id=\"root\"></div>",
        &format!("<div id=\"root\">{body}</div>"),
        1,
    );
    // The viewer draws on the room's own dark; the render page paints paper for a view.
    let page = page.replacen("</head>", "<style>html, body { background: #000; }</style>\n</head>", 1);
    page_with(share, &label, &page, Some(picture))
}

/// The origin a visitor reached this core by — what an absolute `og:image` and the
/// `connect-src` sources are written against.
///
/// **Through the tunnel it is the core's claimed name**, not the request's `Host`: the
/// community forwards the request as plain HTTP/1.1, and a page whose sources name an
/// address the visitor never used would refuse its own requests.
async fn origin(
    data_dir: &std::path::Path,
    relayed: bool,
    headers: &axum::http::HeaderMap,
) -> String {
    if relayed {
        if let Some(named) = super::surfaces::named_base_url(data_dir).await {
            return named;
        }
    }
    let host = headers
        .get(axum::http::header::HOST)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("localhost");
    let scheme = if crate::foundation::surfaces::over_tls(headers) { "https" } else { "http" };
    format!("{scheme}://{host}")
}

/// `GET /<name>` — a shared view or attachment, to somebody who is not the owner.
///
/// Reached from the router's fallback rather than a route of its own, so a share can
/// never shadow something this core serves. `RESERVED` refuses those names at creation
/// too; this is the half that cannot be got wrong by adding a route later.
pub async fn serve(
    axum::extract::State(state): axum::extract::State<std::sync::Arc<super::AppState>>,
    relayed: Option<axum::Extension<crate::foundation::tunnel::Relayed>>,
    uri: axum::http::Uri,
    headers: axum::http::HeaderMap,
) -> axum::response::Response {
    use axum::response::IntoResponse;

    let name = uri.path().trim_start_matches('/').trim_end_matches('/');
    let Some(share) = find(&state.data_dir, name) else {
        return (axum::http::StatusCode::NOT_FOUND, "not found\n").into_response();
    };
    let key = presented_key(&uri, &headers);
    if !share.opened_by(key.as_deref()) {
        // Deliberately the same answer an unshared name gets. A 401 here would confirm
        // the name is real, which is the one thing an unlisted share is hiding.
        return (axum::http::StatusCode::NOT_FOUND, "not found\n").into_response();
    }

    let origin = origin(&state.data_dir, relayed.is_some(), &headers).await;
    // A link preview is fetched by something holding no cookie, so an unlisted share's
    // picture carries the key the way the link does.
    let keyed = |path: String| match (&share.key_hash, &key) {
        (Some(_), Some(key)) => format!("{origin}{path}?key={key}"),
        _ => format!("{origin}{path}"),
    };

    let body = match share.shared() {
        Some(Shared::Attachment(id)) => {
            let (Some(probe), Some(render_page)) =
                (attachments::probe(&state.data_dir, &id).await, crate::appearance::render_page())
            else {
                tracing::warn!(name, "a shared attachment has no object or no render page");
                return (
                    axum::http::StatusCode::SERVICE_UNAVAILABLE,
                    "this page cannot be drawn right now\n",
                )
                    .into_response();
            };
            attachment_page(&share, &id, &probe, &render_page, &keyed(attachments::preview_url(&id)))
        }
        Some(Shared::View(view_ref)) => {
            let Ok(html) = tokio::fs::read_to_string(page_path(&state.data_dir, &view_ref)).await
            else {
                // The record says shared and the page is gone: the check has to run again.
                // Say so rather than serving an empty mount point.
                tracing::warn!(view_ref, "a share has no page on disk");
                return (
                    axum::http::StatusCode::SERVICE_UNAVAILABLE,
                    "this page needs to be published again\n",
                )
                    .into_response();
            };
            let label = view_ref.rsplit('/').next().unwrap_or(&view_ref).replace(['-', '_'], " ");
            let shot = super::view_shots::url_for_ref(&state.data_dir, &view_ref).map(keyed);
            page_with(&share, &label, &html, shot.as_deref())
        }
        None => return (axum::http::StatusCode::NOT_FOUND, "not found\n").into_response(),
    };

    let mut headers_out = axum::http::HeaderMap::new();
    headers_out.insert(
        axum::http::header::CONTENT_TYPE,
        axum::http::HeaderValue::from_static("text/html; charset=utf-8"),
    );
    // CSP limits requests to this core; the share gate makes the per-path decision.
    // A request made after the check (e.g. a lazy picture's about.v1) must not be
    // refused merely because Chrome never happened to ask for it during publication.
    headers_out.insert(
        axum::http::header::CONTENT_SECURITY_POLICY,
        axum::http::HeaderValue::from_static("connect-src 'self'"),
    );
    // An unlisted page is not for a shared cache to keep; a public one may be, and the
    // short life is the whole of what withdrawing it can promise.
    let cache = if share.key_hash.is_some() { "private, max-age=60" } else { "public, max-age=60" };
    if let Ok(v) = axum::http::HeaderValue::from_str(cache) {
        headers_out.insert(axum::http::header::CACHE_CONTROL, v);
    }
    // Hand the key to the browser so the page's own requests carry it. `SameSite=Lax`
    // and no `Domain=`: this is one page on one core, and it travels no further.
    if share.key_hash.is_some() {
        if let Some(key) = key {
            let secure = if crate::foundation::surfaces::over_tls(&headers) { "; Secure" } else { "" };
            if let Ok(v) = axum::http::HeaderValue::from_str(&format!(
                "{SHARE_COOKIE}={key}; HttpOnly; SameSite=Lax; Path=/; Max-Age=3600{secure}"
            )) {
                headers_out.insert(axum::http::header::SET_COOKIE, v);
            }
        }
    }
    (axum::http::StatusCode::OK, headers_out, body).into_response()
}

/// What a share request asks for.
#[derive(serde::Deserialize)]
pub struct ShareRequest {
    /// A view's ref, or an `att:` id.
    #[serde(rename = "ref")]
    pub named: String,
    /// `true` publishes it, `false` withdraws it.
    pub on: bool,
    /// Publish behind a key in the URL rather than openly. Unlisted, not private:
    /// whoever holds the link holds the access, and that is the whole of what it buys.
    #[serde(default)]
    pub unlisted: bool,
    /// One sentence saying what this is — the page's `description` and its link
    /// preview. Read by a person deciding whether to open it and by an agent deciding
    /// whether to read it.
    #[serde(default)]
    pub description: String,
}

/// `POST /api/shares` — publish a view or an attachment as a page, or withdraw it.
///
/// **Publishing a view runs the check and can fail**, which is the difference between this
/// and `bookmark_view`: a bookmark is a preference and always takes, while a share is a claim
/// about a view that has to be true. A view that reads the API renders half-empty to somebody
/// with no session, and the whole point of checking here is that the owner learns that
/// instead of the person they sent it to. `422` carries the reasons, each naming the thing to
/// fix.
pub async fn post_share(
    axum::extract::State(state): axum::extract::State<std::sync::Arc<super::AppState>>,
    super::headers::AuthBearer(auth): super::headers::AuthBearer,
    axum::Json(body): axum::Json<ShareRequest>,
) -> axum::response::Response {
    use axum::http::StatusCode;
    use axum::response::IntoResponse;

    let named = body.named.trim().to_string();
    tracing::info!(auth = ?auth, named = %named, on = body.on, "POST /api/shares");

    if !body.on {
        return match close(&state.data_dir, &named).await {
            Ok(()) => StatusCode::NO_CONTENT.into_response(),
            Err(error) => (StatusCode::BAD_REQUEST, format!("{error:#}")).into_response(),
        };
    }

    match open(&state.data_dir, &named, body.unlisted, &body.description).await {
        Ok(opened) => {
            let (link, reachable) = link(&state.data_dir, &opened.path, opened.key.as_deref()).await;
            (
                StatusCode::OK,
                axum::Json(serde_json::json!({
                    "path": opened.path,
                    "key": opened.key,
                    "link": link,
                    "reachable": reachable,
                })),
            )
                .into_response()
        }
        Err(Refused::Name(why)) => (StatusCode::CONFLICT, why).into_response(),
        Err(Refused::Unknown(why)) => (StatusCode::NOT_FOUND, why).into_response(),
        Err(Refused::Check(reasons)) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            axum::Json(serde_json::json!({ "refusals": reasons })),
        )
            .into_response(),
        Err(Refused::Broken(why)) => {
            tracing::warn!(why, "the share check could not run");
            (StatusCode::INTERNAL_SERVER_ERROR, why).into_response()
        }
    }
}

#[derive(serde::Deserialize)]
pub struct ShareQuery {
    #[serde(rename = "ref")]
    named: String,
}

/// `GET /api/shares?ref=` — whether a view or an attachment is published, and how.
///
/// `404` when it is not. A public share answers with its link, because a public link is
/// only its path and can be handed out again; an unlisted one answers with none, because
/// its key is kept only as a hash and the link it was in cannot be rebuilt — the owner's
/// way to a link they can copy is a new one, which retires the old key.
///
/// Asks the community for this core's name, which is a network round trip, so it is read
/// when an owner opens a share sheet and not on every inventory poll.
pub async fn get_share(
    axum::extract::State(state): axum::extract::State<std::sync::Arc<super::AppState>>,
    super::headers::AuthBearer(auth): super::headers::AuthBearer,
    axum::extract::Query(query): axum::extract::Query<ShareQuery>,
) -> axum::response::Response {
    use axum::http::StatusCode;
    use axum::response::IntoResponse;

    tracing::debug!(auth = ?auth, named = %query.named, "GET /api/shares");
    let name = match Shared::parse(&query.named) {
        Ok(shared) => shared.name(),
        Err(why) => return (StatusCode::BAD_REQUEST, why).into_response(),
    };
    let Some(share) = find(&state.data_dir, &name) else {
        return (StatusCode::NOT_FOUND, "not shared").into_response();
    };
    let (public_link, reachable) = link(&state.data_dir, &format!("/{name}"), None).await;
    let kind = share.kind();
    axum::Json(serde_json::json!({
        "kind": kind,
        "link": (kind == Kind::Public).then_some(public_link),
        "reachable": reachable,
    }))
    .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view_share(view_ref: &str) -> Share {
        Share {
            of: format!("view:{view_ref}"),
            module_url: "/views/_compiled/ab12.mjs".into(),
            key_hash: None,
            created_at: chrono::Utc::now(),
            description: String::new(),
            attachments: Vec::new(),
            resources: Vec::new(),
        }
    }

    const ID: &str = "3f9a0c11d2e4b5a6";

    /// The three things a request can be, and why each matters: the page's own `data:` URLs
    /// reach nothing (a `<video>`'s controls are six of them, and refusing those refused every
    /// view that plays a clip), a path here is what a scope is written in, and another origin
    /// is refused *as a URL* — reading it by its path would let somebody else's machine pass
    /// as this view's own folder.
    #[test]
    fn a_request_is_read_against_the_origin_the_page_was_loaded_from() {
        let here = "http://127.0.0.1:12358";
        assert_eq!(asked("http://127.0.0.1:12358/api/tools", here), Asked::Here("/api/tools".into()));
        assert_eq!(
            asked("http://127.0.0.1:12358/views/x/a.jpg?v=2", here),
            Asked::Here("/views/x/a.jpg".into()),
        );
        assert_eq!(asked("data:image/svg+xml;base64,AAA", here), Asked::Inert);
        assert_eq!(asked("blob:http://127.0.0.1:12358/9d2", here), Asked::Inert);
        assert_eq!(
            asked("https://elsewhere.example/views/x/a.jpg", here),
            Asked::Elsewhere("https://elsewhere.example/views/x/a.jpg".into()),
        );
        // A different port on this machine is a different origin.
        assert!(matches!(asked("http://127.0.0.1:9999/views/x/a.jpg", here), Asked::Elsewhere(_)));
        // And a long one is cut, so a refusal stays readable.
        let long = format!("https://elsewhere.example/{}", "x".repeat(400));
        assert!(matches!(asked(&long, here), Asked::Elsewhere(u) if u.chars().count() == 120));
    }

    /// A view owns only its own resource folder, independent of its nesting depth.
    #[test]
    fn a_view_owns_its_module_and_its_resource_folder() {
        let r = "badminton-top10/leader";
        let m = "/views/_compiled/ab12.mjs";
        assert!(in_scope(m, r, m));
        assert!(in_scope("/assets/share-react.js", r, m));
        assert!(in_scope("/views/badminton-top10/leader.assets/photo.jpg", r, m));
        assert!(in_scope("/views/badminton-top10/leader.assets/figures/one.png", r, m));
        // Its own picture, for the link preview.
        assert!(in_scope("/views/_shots/ref/badminton-top10/leader.png", r, m));

        // Another view's folder, another view's module, and `/views/` at large.
        assert!(!in_scope("/views/autumn-milk-tea/cup.jpg", r, m));
        assert!(!in_scope("/views/badminton-top10/leader.jsx", r, m));
        assert!(!in_scope("/views/badminton-top10/leader.share.json", r, m));
        assert!(!in_scope("/views/badminton-top10/other.jsx", r, m));
        assert!(!in_scope("/views/badminton-top10/other.assets/photo.jpg", r, m));
        assert!(!in_scope("/views/badminton-top10/leader.assets-private/photo.jpg", r, m));
        assert!(!in_scope("/views/_compiled/ff99.mjs", r, m));
        // Somebody else's picture is somebody else's.
        assert!(!in_scope("/views/_shots/ref/autumn-milk-tea/cup.png", r, m));
        assert!(!in_scope("/views/_shots/", r, m));
        assert!(!in_scope("/views/", r, m));
        assert!(!in_scope("/api/views", r, m));
        // The core's own favicon is the core's, not the view's. The page declaring its
        // own icon is what keeps a browser from asking for this at all; widening this
        // list instead would hand a core asset to every visitor of a share.
        assert!(!in_scope("/favicon.ico", r, m));
    }

    /// A view can request declared attachments after the check has finished. Every route
    /// under an authorized id works then, and no other object in the store does.
    #[test]
    fn a_share_serves_declared_attachments_and_no_others() {
        let other = "0000000000000000";
        let share = Share { attachments: vec![ID.into()], ..view_share("court/review") };
        for path in [
            format!("/api/attachments/{ID}"),
            format!("/api/attachments/{ID}/preview.v1"),
            format!("/api/attachments/{ID}/about.v1"),
            format!("/api/attachments/{ID}/playable"),
            format!("/api/attachments/{ID}/proxy.v1"),
        ] {
            assert!(share.serves(&path), "{path}");
        }
        assert!(!share.serves(&format!("/api/attachments/{other}")));
        assert!(!share.serves("/api/attachments/"));
        assert!(!share.serves("/api/attachments/not-an-id/preview.v1"));
        assert!(!view_share("court/review").serves(&format!("/api/attachments/{ID}")));
        // Its own page and its own files, as before.
        assert!(share.serves("/court/review"));
        assert!(share.serves("/views/court/review.assets/x.png"));
        assert!(!share.serves("/views/court/other.jsx"));
        assert!(!share.serves("/api/tools"));
    }

    /// A shared attachment is at `/att/<id>`, and serves its page, its own routes, the stage
    /// module that draws it and the build — nothing of any view.
    #[test]
    fn a_shared_attachment_serves_itself_and_the_viewer() {
        let share = Share {
            of: format!("att:{ID}"),
            module_url: attachments::stage_module_url(ID),
            attachments: vec![ID.into()],
            ..view_share("unused")
        };
        assert_eq!(share.name().as_deref(), Some(format!("att/{ID}").as_str()));
        assert!(share.serves(&format!("/att/{ID}")));
        assert!(share.serves(&format!("/api/attachments/{ID}/stage.v1.mjs")));
        assert!(share.serves(&format!("/api/attachments/{ID}")));
        assert!(share.serves("/assets/render-abc.js"));
        assert!(!share.serves("/views/_compiled/ab12.mjs"));
        assert!(!share.serves("/views/anything/x.png"));
        assert!(!share.serves("/api/attachments/0000000000000000"));
    }

    #[test]
    fn what_is_shared_is_named_the_way_it_is_everywhere() {
        assert_eq!(Shared::parse(&format!("att:{ID}")), Ok(Shared::Attachment(ID.into())));
        assert_eq!(Shared::parse("view:court/review"), Ok(Shared::View("court/review".into())));
        assert_eq!(Shared::parse("court/review"), Ok(Shared::View("court/review".into())));
        assert!(Shared::parse("att:nope").is_err());
        assert!(Shared::parse("").is_err());
        assert_eq!(Shared::Attachment(ID.into()).name(), format!("att/{ID}"));
    }

    /// A view cannot be published under a name this core already answers to. Checked
    /// when the share is made, because a link that has been sent cannot be un-sent.
    #[test]
    fn a_share_cannot_take_a_path_the_core_serves() {
        for taken in ["api", "views", "assets", "up", "render", "healthz", "auth", "att"] {
            assert!(name_collision(taken).is_some(), "{taken} was allowed");
            // The first segment is what decides it, so a project called `api` is
            // refused just as `api` alone is.
            assert!(name_collision(&format!("{taken}/leader")).is_some(), "{taken}/leader");
        }
        assert!(name_collision("agent-arch").is_none());
        assert!(name_collision("badminton-top10/leader").is_none());
        // Only the whole first segment counts — a name that merely starts with a
        // reserved word is a different path and is nobody's but its own.
        assert!(name_collision("api-notes").is_none());
        assert!(name_collision("rendering").is_none());
        assert!(name_collision("attic").is_none());
    }

    /// A public share is opened by anybody; an unlisted one only by its key. The key is
    /// a bearer token in a URL and this is the whole of what it buys — it defeats
    /// enumeration, not sharing.
    #[test]
    fn a_key_is_what_opens_an_unlisted_share() {
        let public = view_share("agent-arch");
        assert!(public.opened_by(None));
        assert!(public.opened_by(Some("anything")));

        let unlisted = Share { key_hash: Some(hex_hash("s3cret")), ..public };
        assert!(unlisted.opened_by(Some("s3cret")));
        assert!(!unlisted.opened_by(Some("s3cre")));
        assert!(!unlisted.opened_by(None), "an unlisted share is not opened by nobody");
    }

    /// The stored key is a hash, so the record can recognise a key it could not
    /// reproduce — the same property a surface credential has, for the same reason.
    #[test]
    fn the_key_is_stored_as_a_hash_and_never_as_itself() {
        let share = Share {
            key_hash: Some(hex_hash("s3cret")),
            description: "a picture of the architecture".into(),
            ..view_share("agent-arch")
        };
        let json = serde_json::to_string(&share).unwrap();
        assert!(!json.contains("s3cret"), "the key was serialised: {json}");
        assert!(json.contains(&hex_hash("s3cret")));

        // A public share carries no key field at all, rather than a null somebody
        // later reads as "there is a key and it is empty".
        let public = Share { key_hash: None, ..share };
        assert!(!serde_json::to_string(&public).unwrap().contains("key_hash"));
    }

    /// The key arrives in the URL the first time and in the cookie thereafter, because
    /// the page's own requests carry no query string. The query wins when both are
    /// present: it is what the person just clicked, and a stale cookie should not
    /// decide what a fresh link opens.
    #[test]
    fn a_key_is_read_from_the_url_or_the_cookie() {
        use axum::http::{HeaderMap, HeaderValue, Uri, header};
        let empty = HeaderMap::new();
        let with_cookie = {
            let mut h = HeaderMap::new();
            h.insert(header::COOKIE, HeaderValue::from_static("a=1; hi_share=fromcookie; b=2"));
            h
        };
        let uri = |s: &str| s.parse::<Uri>().unwrap();

        assert_eq!(presented_key(&uri("/x?key=fromurl"), &empty).as_deref(), Some("fromurl"));
        assert_eq!(presented_key(&uri("/x"), &with_cookie).as_deref(), Some("fromcookie"));
        assert_eq!(
            presented_key(&uri("/x?key=fromurl"), &with_cookie).as_deref(),
            Some("fromurl"),
        );
        assert_eq!(presented_key(&uri("/x"), &empty), None);
        // A query that carries something else is not a key.
        assert_eq!(presented_key(&uri("/x?monkey=no"), &empty), None);
    }

    /// The description and the title go into attributes, so anything that could close
    /// one has to stop being able to. The page is assembled from a person's sentence
    /// and a view's own name, and neither is trusted markup.
    #[test]
    fn the_tags_cannot_break_out_of_their_attributes() {
        let share = Share {
            description: r#"a "chart" of <b>everything</b> & more"#.into(),
            ..view_share("agent-arch")
        };
        let out = page_with(&share, "agent arch", "<html><head></head><body>x</body></html>", None);
        assert!(!out.contains(r#"content="a "chart""#), "the quote was not escaped: {out}");
        assert!(out.contains("&quot;chart&quot;"), "{out}");
        assert!(out.contains("&lt;b&gt;"), "{out}");
        assert!(out.contains("&amp; more"), "{out}");
    }

    /// The tags land inside the head, ahead of `</head>`, and the body is untouched —
    /// the content is the check's output and this only garnishes it. The module the live
    /// half mounts rides with them, because the page's URL carries none.
    #[test]
    fn the_tags_go_in_the_head_and_the_content_is_left_alone() {
        let share = Share { description: "the shape of it".into(), ..view_share("agent-arch") };
        let html = "<html><head><title>hi-agent view render</title></head><body><div id=\"root\">real</div></body></html>";
        let out = page_with(&share, "agent arch", html, Some("https://ana.hi-agent.xyz/views/_shots/ref/agent-arch.png"));
        let head_end = out.find("</head>").expect("still a head");
        assert!(out.find("og:description").unwrap() < head_end);
        assert!(out.find("og:image").unwrap() < head_end);
        assert!(out.contains("content=\"https://ana.hi-agent.xyz/views/_shots/ref/agent-arch.png\""), "{out}");
        assert!(out.contains("<meta name=\"hi-view-module\" content=\"/views/_compiled/ab12.mjs\">"), "{out}");
        assert!(out.contains("<title>the shape of it</title>") && !out.contains("view render"), "{out}");
        assert!(out.contains(">real</div>"), "the content was disturbed: {out}");

        // A capture that is not a document we recognise is served as it is: the
        // content is the point and the tags are the garnish.
        assert_eq!(page_with(&share, "l", "just text", None), "just text");
    }

    /// A shared attachment's page carries the thing itself in its HTML — what a reader that
    /// runs nothing gets — and mounts the host's viewer over it.
    #[test]
    fn a_shared_attachment_is_in_its_own_page() {
        let share = Share {
            of: format!("att:{ID}"),
            module_url: attachments::stage_module_url(ID),
            attachments: vec![ID.into()],
            description: "场地线画回画面".into(),
            ..view_share("unused")
        };
        let render = "<html><head><title>hi-agent view render</title></head><body><div id=\"root\"></div><script type=\"module\" src=\"/assets/render.js\"></script></body></html>";
        let picture = attachments::Probe {
            kind: attachments::Kind::Picture,
            ext: "png".into(),
            bytes: 1,
            width: 1920,
            height: 1080,
            duration_ms: None,
            fps: None,
            codec: None,
            sha256: String::new(),
        };
        let og = format!("https://ana.hi-agent.xyz/api/attachments/{ID}/preview.v1");
        let page = attachment_page(&share, ID, &picture, render, &og);
        assert!(page.contains(&format!("<div id=\"root\"><img src=\"/api/attachments/{ID}\" alt=\"场地线画回画面\"")), "{page}");
        assert!(page.contains(&format!("content=\"/api/attachments/{ID}/stage.v1.mjs\"")), "{page}");
        assert!(page.contains(&format!("<meta property=\"og:image\" content=\"{og}\">")), "{page}");

        let clip = attachments::Probe { kind: attachments::Kind::Clip, ext: "mp4".into(), duration_ms: Some(30_000), ..picture };
        let page = attachment_page(&share, ID, &clip, render, &og);
        assert!(page.contains(&format!("<video src=\"/api/attachments/{ID}/playable\" poster=\"/api/attachments/{ID}/preview.v1\"")), "{page}");
    }

    #[tokio::test]
    async fn view_resources_validate_names_and_existing_attachments() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("views/court")).unwrap();
        assert!(super::super::view_resources::bindings(dir.path(), "court/review").await.unwrap().is_empty());
        std::fs::write(dir.path().join("views/court/review.resources.json"), format!("{{\"hero\":\"att:{ID}\"}}"))
            .unwrap();
        assert!(super::super::view_resources::bindings(dir.path(), "court/review").await.is_err());
        std::fs::write(dir.path().join("views/court/review.resources.json"), "{\"hero\":\"att:bad\"}")
            .unwrap();
        assert!(super::super::view_resources::bindings(dir.path(), "court/review").await.is_err());

        let picture = dir.path().join("picture.png");
        image::RgbImage::from_pixel(4, 4, image::Rgb([20, 130, 80]))
            .save(&picture)
            .unwrap();
        let placed = attachments::place(dir.path(), &picture).await.unwrap();
        std::fs::write(
            dir.path().join("views/court/review.resources.json"),
            format!("{{\"hero\":\"att:{}\"}}", placed.id),
        )
        .unwrap();
        let refs = super::super::view_resources::bindings(dir.path(), "court/review").await.unwrap();
        assert_eq!(refs.get("hero"), Some(&format!("att:{}", placed.id)));
        let share = Share { resources: vec!["hero".into()], ..view_share("court/review") };
        assert!(share.serves("/views/court/review.assets/_resources/hero/about.v1"));
        assert!(!share.serves(&format!("/api/attachments/{}/preview.v1", placed.id)));
        assert!(!share.serves("/api/attachments/1111111111111111/about.v1"));
    }

    /// Single-segment refs also have a resource folder, without exposing siblings.
    #[test]
    fn a_root_ref_owns_only_its_resource_folder_under_views() {
        let m = "/views/_compiled/ab12.mjs";
        assert!(in_scope(m, "agent-arch", m));
        assert!(in_scope("/assets/x.css", "agent-arch", m));
        assert!(in_scope("/views/_shots/ref/agent-arch.png", "agent-arch", m));
        assert!(in_scope("/views/agent-arch.assets/photo.jpg", "agent-arch", m));
        assert!(!in_scope("/views/agent-arch.jpg", "agent-arch", m));
        assert!(!in_scope("/views/anything.jpg", "agent-arch", m));
    }
}
