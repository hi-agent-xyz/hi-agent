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
