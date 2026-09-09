//! The check a view passes before it may be shared, and the three things one run
//! of it produces.
//!
//! A shared view is served to somebody with no session, so it is rendered here the
//! way that person will see it — **with `/api/*` refused** — and the run is read for
//! three separate answers that would otherwise each need a mechanism of their own:
//!
//! | Question | Read from |
//! |---|---|
//! | may this be shared at all? | the page's own verdict, plus a blank frame |
//! | what is the page's HTML? | the settled DOM |
//! | what may it connect to? | what it actually asked for |
//!
//! The first is the one that earns the check. A view that fetches its own data
//! renders half-empty under a share's scope and **looks fine in its source**; without
//! this, the owner is not the one who finds out, the person they sent it to is. See
//! [`docs/arch/sharing.md`](../../../docs/arch/sharing.md).
//!
//! This is the capability, and nothing calls it yet: the share record and the public
//! route are separate slices. Landing it alone is deliberate — it is the half with a
//! browser in it, and it is testable without either.

use crate::body::capabilities::view_render;

/// A request that was refused during the check, as a path on this core.
///
/// Kept as the path rather than the whole URL because that is what a policy is
/// written in, and what a person reading a refusal wants to see.
fn path_of(url: &str) -> String {
    url.split_once("://")
        .and_then(|(_, rest)| rest.split_once('/'))
        .map(|(_, path)| format!("/{path}"))
        .unwrap_or_else(|| url.to_string())
}

/// Whether a URL the page asked for is one a share may serve.
///
/// The list is derived from the ref rather than stored, so it cannot drift from what
/// the view is: this view's own compiled module, this view's own folder, and the
/// build's shared assets. **Never `/views/` at large** — that is one wildcard route
/// with every view's source and every build artifact behind it, and opening it would
/// hand over the workshop to share one poster.
fn in_scope(path: &str, view_ref: &str, module_url: &str) -> bool {
    if path == module_url {
        return true;
    }
    if path.starts_with("/assets/") {
        return true;
    }
    // The view's own folder: `badminton-top10/leader` owns `/views/badminton-top10/`.
    // A single-segment ref owns nothing under `/views/`, because it has no folder of
    // its own to own — its files would be siblings of every other view's.
    match view_ref.rsplit_once('/') {
        Some((project, _)) => path.starts_with(&format!("/views/{project}/")),
        None => false,
    }
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
    /// What the page may connect to, as paths — the `connect-src` this share should
    /// carry. Empty means `'none'`, which is the ordinary case for a view that holds
    /// its own data.
    pub connect_src: Vec<String>,
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
    let mut connect_src = Vec::new();
    for url in &rendered.requested {
        let path = path_of(url);
        if path == "/render/view" || path.starts_with("/render/view?") {
            continue; // the host page itself
        }
        if in_scope(&path, view_ref, module_url) {
            if !connect_src.contains(&path) {
                connect_src.push(path);
            }
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
        connect_src,
        png: rendered.png,
    })
}

/// `app_settings` key holding the shares, as a JSON array.
///
/// **In the config store, not the views tree**, for the reason bookmarks are: the tree
/// is disposable and `factory/` is re-seeded on every boot, and a share is a decision
/// the person made about publishing something. It has to outlive an upgrade.
const SHARES_KEY: &str = "view_shares";

/// Where a share's rendered page lives — a tool dir inside the views tree, beside
/// `_compiled/` and `_shots/`, and disposable in the same way.
///
/// The page is the *check's output*, so losing it means the view has to be checked
/// again before it can be served — which is the correct consequence, not a bug to
/// paper over with a fallback that serves a mount point with no content in it.
fn shares_dir(data_dir: &std::path::Path) -> std::path::PathBuf {
    data_dir.join("views").join("_shares")
}

fn page_path(data_dir: &std::path::Path, view_ref: &str) -> std::path::PathBuf {
    shares_dir(data_dir).join(format!("{view_ref}.html"))
}

/// One published view.
///
/// The key is stored as a hash for the same reason a credential is: it is a bearer
/// token, it exists in plaintext exactly once, and what is kept here is only enough to
/// recognise it. `None` is a public share.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Share {
    pub view_ref: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key_hash: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    /// What this is, in a sentence — the `<meta name="description">` and the
    /// `og:description`. Written by the agent at share time, because it is the half of
    /// the page a reader sees before deciding to open it.
    #[serde(default)]
    pub description: String,
    /// What the page may connect to, read off the check rather than declared.
    #[serde(default)]
    pub connect_src: Vec<String>,
}

impl Share {
    /// Whether `presented` opens this share. A public share is opened by anybody.
    pub fn opened_by(&self, presented: Option<&str>) -> bool {
        match &self.key_hash {
            None => true,
            Some(want) => presented.is_some_and(|key| &hex_hash(key) == want),
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

/// The share published at `view_ref`, if any.
pub fn find(data_dir: &std::path::Path, view_ref: &str) -> Option<Share> {
    read_shares(data_dir).into_iter().find(|s| s.view_ref == view_ref)
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
const RESERVED: &[&str] = &[
    "api", "views", "assets", "generated", "up", "render", "auth", "account", "inspect",
    "healthz", "mcp", "favicon.ico", "robots.txt", "index.html", "static", "well-known",
    ".well-known", "sw.js", "manifest.json",
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

/// Why a view could not be shared.
#[derive(Debug, Clone)]
pub enum Refused {
    /// The name collides with something this core serves.
    Name(String),
    /// The check ran and the view did not pass it.
    Check(Vec<String>),
    /// The check could not run at all.
    Broken(String),
}

/// Check `view_ref` and, if it passes, publish it.
///
/// **The check is the only path to a share record** — there is no argument for
/// publishing something nobody has looked at, and this is the function that makes that
/// structural rather than a rule somebody has to remember.
pub async fn open(
    data_dir: &std::path::Path,
    view_ref: &str,
    unlisted: bool,
    description: &str,
) -> Result<Opened, Refused> {
    if let Some(why) = name_collision(view_ref) {
        return Err(Refused::Name(why));
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

    let checked =
        check(view_ref, &module_url).await.map_err(|e| Refused::Broken(format!("{e:#}")))?;
    if !checked.ok {
        return Err(Refused::Check(checked.refusals));
    }
    let Some(html) = checked.html else {
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

    let key = unlisted.then(crate::foundation::surfaces::random_token);
    let share = Share {
        view_ref: view_ref.to_string(),
        key_hash: key.as_deref().map(hex_hash),
        created_at: chrono::Utc::now(),
        description: description.trim().to_string(),
        connect_src: checked.connect_src,
    };

    let mut shares = read_shares(data_dir);
    shares.retain(|s| s.view_ref != view_ref);
    shares.push(share);
    write_shares(data_dir, &shares).map_err(|e| Refused::Broken(format!("{e:#}")))?;

    Ok(Opened { path: format!("/{view_ref}"), key })
}

/// Stop publishing `view_ref`. Idempotent, and it takes the page with it — a page left
/// behind is a file that still answers to anyone who reaches it directly.
///
/// **The link keeps working until the edge forgets it**, which is a property of serving
/// uncredentialed pages and not something this can fix. Say so where a person can read
/// it rather than implying otherwise.
pub async fn close(data_dir: &std::path::Path, view_ref: &str) -> anyhow::Result<()> {
    let mut shares = read_shares(data_dir);
    shares.retain(|s| s.view_ref != view_ref);
    write_shares(data_dir, &shares)?;
    let _ = tokio::fs::remove_file(page_path(data_dir, view_ref)).await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_url_is_read_as_a_path_on_this_core() {
        assert_eq!(path_of("http://127.0.0.1:12358/api/tools"), "/api/tools");
        assert_eq!(path_of("https://ana.hi-agent.xyz/views/x/a.jpg"), "/views/x/a.jpg");
        // Something with no path at all, and something that is not a URL: both come
        // back unchanged rather than being repaired into a path that was never there.
        assert_eq!(path_of("http://127.0.0.1:12358"), "http://127.0.0.1:12358");
        assert_eq!(path_of("data:image/png;base64,AAA"), "data:image/png;base64,AAA");
    }

    /// The scope is the view's own module, the build's assets, and the view's own
    /// folder — and the folder half is why a two-segment ref can carry pictures.
    #[test]
    fn a_view_owns_its_module_its_folder_and_the_shared_assets() {
        let r = "badminton-top10/leader";
        let m = "/views/_compiled/ab12.mjs";
        assert!(in_scope(m, r, m));
        assert!(in_scope("/assets/share-react.js", r, m));
        assert!(in_scope("/views/badminton-top10/leader.jpg", r, m));

        // Another view's folder, another view's module, and the workshop at large.
        assert!(!in_scope("/views/autumn-milk-tea/cup.jpg", r, m));
        assert!(!in_scope("/views/_compiled/ff99.mjs", r, m));
        assert!(!in_scope("/views/", r, m));
        assert!(!in_scope("/api/views", r, m));
    }

    /// A view cannot be published under a name this core already answers to. Checked
    /// when the share is made, because a link that has been sent cannot be un-sent.
    #[test]
    fn a_share_cannot_take_a_path_the_core_serves() {
        for taken in ["api", "views", "assets", "up", "render", "healthz", "auth"] {
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
    }

    /// A public share is opened by anybody; an unlisted one only by its key. The key is
    /// a bearer token in a URL and this is the whole of what it buys — it defeats
    /// enumeration, not sharing.
    #[test]
    fn a_key_is_what_opens_an_unlisted_share() {
        let public = Share {
            view_ref: "agent-arch".into(),
            key_hash: None,
            created_at: chrono::Utc::now(),
            description: String::new(),
            connect_src: Vec::new(),
        };
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
            view_ref: "agent-arch".into(),
            key_hash: Some(hex_hash("s3cret")),
            created_at: chrono::Utc::now(),
            description: "a picture of the architecture".into(),
            connect_src: Vec::new(),
        };
        let json = serde_json::to_string(&share).unwrap();
        assert!(!json.contains("s3cret"), "the key was serialised: {json}");
        assert!(json.contains(&hex_hash("s3cret")));

        // A public share carries no key field at all, rather than a null somebody
        // later reads as "there is a key and it is empty".
        let public = Share { key_hash: None, ..share };
        assert!(!serde_json::to_string(&public).unwrap().contains("key_hash"));
    }

    /// A single-segment ref has no folder of its own, so it owns nothing under
    /// `/views/` — its neighbours there are every other view's files.
    #[test]
    fn a_ref_with_no_folder_owns_nothing_under_views() {
        let m = "/views/_compiled/ab12.mjs";
        assert!(in_scope(m, "agent-arch", m));
        assert!(in_scope("/assets/x.css", "agent-arch", m));
        assert!(!in_scope("/views/agent-arch.jpg", "agent-arch", m));
        assert!(!in_scope("/views/anything.jpg", "agent-arch", m));
    }
}
