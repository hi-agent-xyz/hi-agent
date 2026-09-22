//! Signing a URL the edge will accept: EdgeOne token authentication, type A.
//!
//! `<path>?<param>=<ts>-<rand>-<uid>-<md5("<path>-<ts>-<rand>-<uid>-<key>")>`,
//! where `ts` is when the URL was signed and the edge accepts it until
//! `ts + valid`. The edge strips the parameter before it looks in its cache, so
//! every signature for one path shares one cached object.
//!
//! **Root-relative, with no host at all**: the edge answers `/cache/*` on the
//! core's own origin, so a redirect never leaves it (`docs/arch/topology.md`
//! § *Content*).

use std::time::Duration;

use md5::{Digest as _, Md5};

/// The read half of what the broker hands a core: the key the edge checks
/// signatures against, and how.
///
/// **One key for the whole domain, and that is a named loan** — every core holds a
/// key that can sign a URL for any path, and the key layout is public. What takes
/// it back is per-handle keys validated by an edge function; the loan expires on
/// the second user. See `docs/arch/topology.md` § *Content*.
#[derive(Clone)]
pub struct ReadKey {
    /// The query parameter the edge reads the token from.
    pub param: String,
    pub key: String,
    /// The edge's "effective duration": how long after signing a URL is accepted.
    pub valid: Duration,
}

impl std::fmt::Debug for ReadKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReadKey")
            .field("param", &self.param)
            .field("valid", &self.valid)
            .finish_non_exhaustive()
    }
}

impl ReadKey {
    /// The root-relative URL for `object_path` — the path at the edge, leading slash
    /// included — signed at `now` (unix seconds).
    ///
    /// **The signing time is rounded down to a quarter of the validity**, and the
    /// random part is a constant. Both are there so that one path yields one URL
    /// for hours at a time: a browser caches the object under the whole URL, query
    /// and all, so a fresh signature per redirect would be a fresh download per
    /// redirect of something it already has. Nothing is lost by it — the token is
    /// bound to its path and expires either way, and type A tracks no nonces.
    pub fn signed_url(&self, object_path: &str, now: i64) -> String {
        let window = (self.valid.as_secs() as i64 / 4).max(1);
        let ts = now - now.rem_euclid(window);
        format!("{object_path}?{}={}", self.param, token(object_path, ts, "0", "0", &self.key))
    }

    /// How long a browser may keep the redirect to a signed URL.
    ///
    /// Half the validity: a URL is signed up to a quarter of the validity *before*
    /// the redirect carrying it is issued, so a redirect replayed from the browser
    /// cache at the end of its life still names a URL with a quarter left to run.
    pub fn redirect_max_age(&self) -> u64 {
        self.valid.as_secs() / 2
    }
}

/// `<ts>-<rand>-<uid>-<md5hash>`, the value of the token parameter.
fn token(path: &str, ts: i64, rand: &str, uid: &str, key: &str) -> String {
    let digest = Md5::digest(format!("{path}-{ts}-{rand}-{uid}-{key}").as_bytes());
    format!("{ts}-{rand}-{uid}-{}", super::hex(&digest))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The worked example in EdgeOne's own documentation for type A, so this is
    /// checked against the edge's arithmetic rather than against itself.
    #[test]
    fn token_matches_the_documented_example() {
        assert_eq!(
            token("/foo.jpg", 1721028437, "Kv4cPTAAP5YTi", "0", "DvYmqE81E1F9R791H6lmht"),
            "1721028437-Kv4cPTAAP5YTi-0-0fbdca749d7ab784750685347e42075c"
        );
    }

    fn key() -> ReadKey {
        ReadKey {
            param: "auth_key".into(),
            key: "k".into(),
            valid: Duration::from_secs(86_400),
        }
    }

    /// One path, one URL, for a whole window — which is what lets the browser's
    /// own cache answer the second look.
    #[test]
    fn a_path_signs_to_one_url_within_a_window() {
        let k = key();
        let a = k.signed_url("/cache/ana/api/media/file/x.jpg", 1_800_000_000);
        let b = k.signed_url("/cache/ana/api/media/file/x.jpg", 1_800_000_000 + 60);
        assert_eq!(a, b);
        assert!(a.starts_with("/cache/ana/api/media/file/x.jpg?auth_key="));
    }

    /// Whatever the rounding, a redirect replayed at the very end of its cache life
    /// must still name a URL the edge accepts.
    #[test]
    fn a_cached_redirect_never_outlives_its_signature() {
        let k = key();
        let valid = k.valid.as_secs() as i64;
        for now in [1_800_000_000i64, 1_800_000_000 + valid / 4 - 1, 1_800_021_599] {
            let url = k.signed_url("/p", now);
            let ts: i64 = url.split('=').nth(1).unwrap().split('-').next().unwrap().parse().unwrap();
            let replayed_at = now + k.redirect_max_age() as i64;
            assert!(ts + valid > replayed_at, "signed at {ts}, replayed at {replayed_at}");
        }
    }
}
