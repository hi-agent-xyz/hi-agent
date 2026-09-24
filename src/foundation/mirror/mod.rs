//! Mirroring — immutable bytes leave this machine ahead of being asked for, so the
//! edge in front of `<handle>.hi-agent.xyz` can answer them at this core's own
//! paths, from the community's bucket, instead of through the tunnel.
//!
//! See `docs/arch/cache.md`. The short of it: the relay is an 11 Mbps box, a
//! conversation fits through it and a photograph does not. So an edge function
//! sits in front of the fronted paths and, for a session cookie this core signed,
//! answers from the bucket what is there; anything else comes on to the core, as it
//! always did. **This core never redirects** — a page asks the same URL on
//! loopback, on a custom domain, and through the community.
//!
//! **Two ways a copy is trusted.** Under `/api/media/*` and `/api/attachments/*`
//! the bytes never change, so the edge serves a copy on its own word and this core
//! never hears of it. Under `/views/*` and `/api/drive/file/*` an agent rewrites
//! files in place, so the edge asks first — the browser's request plus
//! `x-hi-cache: ask` — and [`layer`] answers `304` with `x-hi-cache: bucket` when
//! the copy it put up carries the `ETag` the route would send now. That makes a
//! stale copy impossible and costs a headers-only round trip.
//!
//! This module is the core's whole part in that, and it has two jobs:
//!
//! - **Hold the keys.** One answer from the community carries both: `K_handle`,
//!   which [`crate::foundation::surfaces`] signs `hi_surface` with ([`session_key`]),
//!   and an hour's STS credential that can only put objects under `cache/<handle>/`.
//!   Asked for when there is something to upload or no key in hand, never on a
//!   clock, and kept only in memory. **A refusal withdraws both.**
//! - **Upload.** At the write ([`enqueue`]), for what a person handed over and for
//!   an attachment at placement; and on a **miss** ([`layer`]) — a relayed request
//!   carrying a token signed under this core's current key, for an immutable path,
//!   reaching the core at all, means the edge looked and did not find it; for a
//!   changing path, an `ask` this core could not answer with `bucket` is the same.
//!
//! ## What may be mirrored: the response already says
//!
//! **Trusted: a response whose `Cache-Control` says `immutable`. Checked: a
//! response with an `ETag`, of at least [`CHECKED_FLOOR`].** Both under a fronted
//! path, at a plain-ASCII path. Not a list of routes: the one judgment a person has
//! to make — *do these bytes ever change?* — is the one that header already asks,
//! and a route that forgets to say so costs acceleration and nothing else. The key
//! is `cache/<handle>/<request path>`, which the edge builds from the host and the
//! path it was asked for; plain ASCII so the two agree byte for byte.
//!
//! **The bucket's copy outlives what the core would still serve** — a faded day,
//! a deleted file — until its lifecycle drops it. That is accepted
//! (`docs/arch/cache.md` § *What this accepts*), and nothing here deletes.

mod cos;
mod credential;
mod record;

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, RwLock};
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::extract::Request;
use axum::body::HttpBody as _;
use axum::http::header::{CACHE_CONTROL, CONTENT_LENGTH, CONTENT_RANGE, CONTENT_TYPE, ETAG};
use axum::http::{HeaderMap, HeaderName, HeaderValue, Method, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse as _, Response};
use bytes::{Bytes, BytesMut};
use futures::StreamExt as _;
use tokio::sync::mpsc;
use tower::ServiceExt as _;

use crate::foundation::surfaces::{self, Acceptor};
use crate::foundation::tunnel::{self, Relayed};
use credential::{Answer, Grant};
use record::{Records, Row};

/// The size above which an upload goes in parts. Each part is retried on its own,
/// so a home connection that drops mid-video loses one part, not the video.
///
/// **That is the whole of the resumability today.** An upload cut off by a restart,
/// or abandoned after a part failed three times, starts again from nothing on the
/// next miss; the parts it left are removed by the bucket's rule for incomplete
/// uploads. Resuming across a restart would mean keeping the upload id and part
/// list in the record, which nothing does yet.
const PART: usize = 8 * 1024 * 1024;

/// Renew the write key this long before it runs out, so a part is never sent on
/// a key that expires while it is in flight.
const WRITE_KEY_MARGIN: chrono::Duration = chrono::Duration::minutes(5);

/// How long to stop asking after the community declined, or could not be reached.
/// Only demand asks at all — something to upload, or a relayed request while no
/// key is in hand — so this bounds how often demand turns into a call, not a timer.
const DECLINED_FOR: Duration = Duration::from_secs(15 * 60);
const UNREACHABLE_FOR: Duration = Duration::from_secs(5 * 60);

/// A row stops vouching this long before the bucket's lifecycle would drop its
/// object, for however the bucket rounds its expiry to a day.
const LIFECYCLE_SLACK: i64 = 86_400;

/// The smallest checked object worth copying. Each checked hit costs a round trip to
/// this core *and* a bucket read; below this the tunnel's own bytes are cheaper.
/// **A guess, not a measurement** — `docs/arch/cache.md` § *Open* 3.
const CHECKED_FLOOR: u64 = 256 * 1024;

/// The request header the edge asks with — `ask`, or `missing` when the bucket did
/// not hold what this core vouched for — and the response header this core answers
/// `bucket` in. `edge/cache.js` in the site repo reads and writes the same name.
const X_HI_CACHE: &str = "x-hi-cache";

static MIRROR: OnceLock<Mirror> = OnceLock::new();

/// How the edge will trust a copy of a path — which decides what the uploader
/// requires of the response before it copies it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// Served on the edge's word alone: the response must say `immutable`.
    Trusted,
    /// Served only after this core says the copy is current: the response must carry
    /// an `ETag`, and be at least [`CHECKED_FLOOR`].
    Checked,
}

struct Mirror {
    data_dir: PathBuf,
    records: Records,
    /// The keys, once the community has handed them over. Read on every relayed
    /// request and every cookie the gate checks, written about once an hour.
    grant: RwLock<Option<Arc<Grant>>>,
    queue: mpsc::UnboundedSender<Job>,
    /// Paths queued and not yet done, so a page of forty pictures asked for twice
    /// queues forty uploads, not eighty.
    pending: Mutex<HashSet<String>>,
    /// A [`Job::Keys`] is queued and not yet done.
    asking: AtomicBool,
    /// Unix seconds before which asking again is pointless.
    quiet_until: AtomicI64,
}

enum Job {
    /// Put this request path in the bucket, if it wants to be there.
    Mirror(String, Mode),
    /// Get keys: none are in hand, or the write key has run out.
    Keys,
}

/// Start mirroring for this core: open the record and run the uploader.
///
/// `router` is the whole router, gate included; the uploader fetches each object
/// through it, marked as loopback, so what goes into the bucket is exactly the
/// response this core would give — headers and all — with no second way of
/// finding a file on disk to keep in agreement with the first.
pub fn start(data_dir: &Path, router: Router) {
    let records = match Records::open(data_dir) {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!(error = %format!("{e:#}"), "mirror record unavailable; every object is served from here");
            return;
        }
    };
    let (queue, rx) = mpsc::unbounded_channel();
    let mirror = Mirror {
        data_dir: data_dir.to_path_buf(),
        records,
        grant: RwLock::new(None),
        queue,
        pending: Mutex::new(HashSet::new()),
        asking: AtomicBool::new(false),
        quiet_until: AtomicI64::new(0),
    };
    if MIRROR.set(mirror).is_err() {
        return;
    }
    tokio::spawn(work(router, rx));
}

/// The key this core signs session cookies with, while the community grants one.
///
/// `None` before the first grant, after a refusal, and on every core the community
/// is not in front of — whose sessions are then plain ids, as they always were.
pub fn session_key() -> Option<Arc<[u8]>> {
    MIRROR.get()?.grant().map(|g| g.sign_key.clone())
}

/// Queue `path` — a request path this core serves — to be mirrored.
///
/// What a write calls, right after the bytes are durable: **the trigger is the
/// core writing the file, not someone asking for it.** A photo that arrives is
/// uploaded while nobody is looking, so the first look — the common one in a life
/// record — already finds it at the edge.
///
/// Free when mirroring is off, and harmless for a path that turns out not to be
/// mirrorable: the uploader asks the route and drops what it will not vouch for.
pub fn enqueue(path: &str) {
    queue(path, Mode::Trusted);
}

fn queue(path: &str, mode: Mode) {
    let Some(m) = MIRROR.get() else { return };
    if m.quiet() || !signable(path) {
        return;
    }
    let fresh = m.pending.lock().unwrap_or_else(|e| e.into_inner()).insert(path.to_string());
    if fresh {
        let _ = m.queue.send(Job::Mirror(path.to_string(), mode));
    }
}

/// The layer: between the gate and the routes, and only for what came through the
/// tunnel — nothing else passed an edge that could have answered it.
///
/// A relayed request asks for keys when none are in hand, so a core that restarted
/// signs again from about its first request. Then, for a `GET` or `HEAD` on a fronted
/// path whose session is signed under this core's current key:
///
/// - **no `x-hi-cache`** — the edge trusts this path and looked in the bucket
///   already, so reaching the core at all is a miss: an `immutable` answer goes up;
/// - **`x-hi-cache: ask`** — the edge checks this path with the core. If what went up
///   carries the `ETag` the route answered with now, the answer becomes a bodiless
///   `304` with `x-hi-cache: bucket`, and the edge serves the bytes from the bucket.
///   Otherwise the bytes go back as they are, and this version goes up behind them;
/// - **`x-hi-cache: missing`** — the bucket did not hold what this core vouched for:
///   forget the record, serve, and put it back.
///
/// The route runs first either way, so the gate, the route's own `404` and the
/// browser's own `304` all reach the edge unchanged.
pub async fn layer(req: Request, next: Next) -> Response {
    let Some(m) = MIRROR.get().filter(|_| req.extensions().get::<Relayed>().is_some()) else {
        return next.run(req).await;
    };
    m.want_keys();
    let path = req.uri().path().to_string();
    let signed = matches!(*req.method(), Method::GET | Method::HEAD)
        && m.grant().is_some_and(|g| g.fronts(&path) && surfaces::signed_session(req.headers(), &g.sign_key));
    if !signed {
        return next.run(req).await;
    }
    let asked = header(req.headers(), HeaderName::from_static(X_HI_CACHE));
    let resp = next.run(req).await;
    if !matches!(resp.status(), StatusCode::OK | StatusCode::PARTIAL_CONTENT) {
        return resp;
    }
    match asked.as_str() {
        "ask" | "missing" => {
            let Some(etag) = resp.headers().get(ETAG).cloned() else { return resp };
            if asked == "missing" {
                m.records.forget(&path);
            } else if m.vouches(&path, etag.to_str().unwrap_or("")) {
                return in_bucket(etag);
            }
            if whole_len(&resp).is_some_and(|len| len >= CHECKED_FLOOR) {
                queue(&path, Mode::Checked);
            }
        }
        _ if immutable(resp.headers()) => enqueue(&path),
        _ => {}
    }
    resp
}

/// The answer that sends the edge to the bucket: no body, the version the copy is.
fn in_bucket(etag: HeaderValue) -> Response {
    let mut r = StatusCode::NOT_MODIFIED.into_response();
    r.headers_mut().insert(ETAG, etag);
    r.headers_mut().insert(HeaderName::from_static(X_HI_CACHE), HeaderValue::from_static("bucket"));
    r
}

impl Mirror {
    fn grant(&self) -> Option<Arc<Grant>> {
        self.grant.read().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// Whether the bucket holds `path` at version `etag`, under the keys in hand.
    fn vouches(&self, path: &str, etag: &str) -> bool {
        let now = chrono::Utc::now().timestamp();
        !etag.is_empty()
            && self.grant().is_some_and(|g| {
                self.records.get(path).is_some_and(|row| row.etag == etag && fresh(&row, &g, now))
            })
    }

    fn quiet(&self) -> bool {
        chrono::Utc::now().timestamp() < self.quiet_until.load(Ordering::Relaxed)
    }

    fn quiet_for(&self, d: Duration) {
        let until = chrono::Utc::now().timestamp() + d.as_secs() as i64;
        self.quiet_until.store(until, Ordering::Relaxed);
    }

    /// Queue a request for keys if none are in hand or the write key has run out —
    /// the second is also how a refusal, a rotated key or a renamed handle reaches
    /// a core that is only being looked at, about once an hour.
    fn want_keys(&self) {
        let stale = self.grant().is_none_or(|g| g.write.expires_at - chrono::Utc::now() <= WRITE_KEY_MARGIN);
        if stale && !self.quiet() && !self.asking.swap(true, Ordering::Relaxed) {
            let _ = self.queue.send(Job::Keys);
        }
    }

    /// The keys, with a write key that has a while left to run: the ones in hand if
    /// they are for this handle, otherwise fresh ones. `None` when there is nothing
    /// to upload for — no handle served, or the community declined.
    async fn keys(&self) -> Option<Arc<Grant>> {
        let now = chrono::Utc::now();
        if self.quiet() {
            return None;
        }
        // Nobody can reach this core by name, so no edge is in front of it.
        let Some(handle) = tunnel::chosen(&self.data_dir).filter(|_| tunnel::on(&self.data_dir))
        else {
            self.quiet_for(DECLINED_FOR);
            return None;
        };
        if let Some(g) = self.grant() {
            if g.handle == handle && g.write.expires_at - now > WRITE_KEY_MARGIN {
                return Some(g);
            }
        }
        match credential::fetch(&self.data_dir, &handle).await {
            Ok(Answer::Granted(g)) => {
                let g = Arc::new(g);
                self.records.purge(now.timestamp() - g.lifetime.as_secs() as i64);
                *self.grant.write().unwrap_or_else(|e| e.into_inner()) = Some(g.clone());
                Some(g)
            }
            Ok(Answer::Declined(why)) => {
                // Declining to mint is how a core's access is withdrawn, so the sign
                // key goes too: no session is signed on the strength of old keys.
                tracing::info!(%handle, %why, "the community declined a cache grant; serving every object from here");
                *self.grant.write().unwrap_or_else(|e| e.into_inner()) = None;
                self.quiet_for(DECLINED_FOR);
                None
            }
            Err(e) => {
                // Unreachable is not declined: keep signing with the key in hand, and
                // try again shortly.
                tracing::warn!(%handle, error = %format!("{e:#}"), "could not get a cache grant");
                self.quiet_for(UNREACHABLE_FOR);
                None
            }
        }
    }

    /// Mirror one path, if it wants mirroring and the keys are to hand.
    async fn mirror_one(&self, router: &Router, path: &str, mode: Mode) -> anyhow::Result<()> {
        let Some(grant) = self.keys().await else { return Ok(()) };
        let now = chrono::Utc::now().timestamp();
        let row = self.records.get(path).filter(|row| fresh(row, &grant, now));
        if !grant.fronts(path) || (mode == Mode::Trusted && row.is_some()) {
            return Ok(());
        }

        let mut req = Request::get(path).body(Body::empty())?;
        req.extensions_mut().insert(Acceptor::Loopback);
        let resp = router.clone().oneshot(req).await.unwrap_or_else(|e| match e {});
        let etag = header(resp.headers(), ETAG);
        let wanted = match mode {
            Mode::Trusted => immutable(resp.headers()),
            Mode::Checked => {
                !etag.is_empty()
                    && row.is_none_or(|row| row.etag != etag)
                    && whole_len(&resp).is_some_and(|len| len >= CHECKED_FLOOR)
            }
        };
        if resp.status() != StatusCode::OK || !wanted {
            return Ok(());
        }
        let content_type = header(resp.headers(), CONTENT_TYPE);
        let cache_control = header(resp.headers(), CACHE_CONTROL);
        let meta = cos::Meta { content_type: &content_type, cache_control: &cache_control };

        let key = grant.key(path);
        let target = grant.target();
        let bucket = Renewing { mirror: self, current: tokio::sync::Mutex::new(None), grant };
        let total = upload(&bucket, &key, &meta, resp.into_body()).await?;
        self.records.uploaded(path, &target, chrono::Utc::now().timestamp(), &etag);
        tracing::info!(path, bytes = total, ?mode, "mirrored");
        Ok(())
    }
}

/// The five calls an upload makes, so the part-splitting can be tested against
/// something that is not the network.
trait Bucket {
    async fn put(&self, key: &str, meta: &cos::Meta<'_>, body: Bytes) -> anyhow::Result<()>;
    async fn initiate(&self, key: &str, meta: &cos::Meta<'_>) -> anyhow::Result<String>;
    async fn part(&self, key: &str, id: &str, n: u32, body: Bytes) -> anyhow::Result<String>;
    async fn complete(&self, key: &str, id: &str, parts: &[(u32, String)]) -> anyhow::Result<()>;
    async fn abort(&self, key: &str, id: &str);
}

/// Upload a response body under `key`, returning its length: in one request if it
/// ends before a part fills, in parts otherwise. A multipart upload that fails is
/// aborted, so its parts stop being stored.
async fn upload(bucket: &impl Bucket, key: &str, meta: &cos::Meta<'_>, body: Body) -> anyhow::Result<u64> {
    let mut stream = body.into_data_stream();
    let mut buf = BytesMut::new();
    let mut total = 0u64;
    let mut multipart: Option<(String, Vec<(u32, String)>)> = None;

    let result: anyhow::Result<()> = async {
        loop {
            let done = match stream.next().await {
                Some(chunk) => {
                    let chunk = chunk?;
                    total += chunk.len() as u64;
                    buf.extend_from_slice(&chunk);
                    false
                }
                None => true,
            };
            // Full parts go as they fill; the tail goes once the body ends, but only
            // if this already became a multipart upload.
            while buf.len() >= PART || (done && multipart.is_some() && !buf.is_empty()) {
                let part = buf.split_to(buf.len().min(PART)).freeze();
                if multipart.is_none() {
                    let id = retry(|| bucket.initiate(key, meta)).await?;
                    multipart = Some((id, Vec::new()));
                }
                let Some((id, parts)) = multipart.as_mut() else { unreachable!() };
                let n = parts.len() as u32 + 1;
                let etag = retry(|| bucket.part(key, id, n, part.clone())).await?;
                parts.push((n, etag));
            }
            if done {
                break;
            }
        }
        match &multipart {
            None => retry(|| bucket.put(key, meta, buf.clone().freeze())).await,
            Some((id, parts)) => retry(|| bucket.complete(key, id, parts)).await,
        }
    }
    .await;

    if let Err(e) = result {
        if let Some((id, _)) = &multipart {
            bucket.abort(key, id).await;
        }
        return Err(e);
    }
    Ok(total)
}

/// The real bucket, with the write key renewed between calls when it is about to
/// run out — a long video on a slow uplink can outlast the hour a key is good for.
struct Renewing<'a> {
    mirror: &'a Mirror,
    grant: Arc<Grant>,
    /// The client for the grant in use, rebuilt only when the key is.
    current: tokio::sync::Mutex<Option<(Arc<Grant>, Arc<cos::Client>)>>,
}

impl Renewing<'_> {
    async fn client(&self) -> anyhow::Result<Arc<cos::Client>> {
        let lasts = |g: &Grant| g.write.expires_at - chrono::Utc::now() > WRITE_KEY_MARGIN;
        let mut current = self.current.lock().await;
        let grant = match current.as_ref() {
            Some((g, c)) if lasts(g) => return Ok(c.clone()),
            None if lasts(&self.grant) => self.grant.clone(),
            _ => self
                .mirror
                .keys()
                .await
                .ok_or_else(|| anyhow::anyhow!("the write key ran out and no new one was granted"))?,
        };
        let client = Arc::new(cos::Client::new(&grant.bucket, &grant.region, grant.write.clone()));
        *current = Some((grant, client.clone()));
        Ok(client)
    }
}

impl Bucket for Renewing<'_> {
    async fn put(&self, key: &str, meta: &cos::Meta<'_>, body: Bytes) -> anyhow::Result<()> {
        self.client().await?.put(key, meta, body).await
    }
    async fn initiate(&self, key: &str, meta: &cos::Meta<'_>) -> anyhow::Result<String> {
        self.client().await?.initiate(key, meta).await
    }
    async fn part(&self, key: &str, id: &str, n: u32, body: Bytes) -> anyhow::Result<String> {
        self.client().await?.upload_part(key, id, n, body).await
    }
    async fn complete(&self, key: &str, id: &str, parts: &[(u32, String)]) -> anyhow::Result<()> {
        self.client().await?.complete(key, id, parts).await
    }
    /// With the client already in hand, not a renewed one: an abort usually follows a
    /// failure, and renewing may be the thing that failed. If the key has run out the
    /// abort is refused, and the bucket's rule for incomplete uploads removes the parts.
    async fn abort(&self, key: &str, id: &str) {
        let current = self.current.lock().await.as_ref().map(|(_, c)| c.clone());
        if let Some(c) = current {
            c.abort(key, id).await;
        }
    }
}

/// The uploader: one job at a time, in the order they came.
///
/// **One at a time is the whole of "low priority" today.** The design asks for
/// uploads to run when the uplink is otherwise idle, and nothing here measures
/// that yet: a large upload does share the uplink with the tunnel while it runs.
/// Sequential bounds that to one stream; watching the tunnel's own traffic is what
/// would finish the job.
async fn work(router: Router, mut rx: mpsc::UnboundedReceiver<Job>) {
    let Some(m) = MIRROR.get() else { return };
    while let Some(job) = rx.recv().await {
        match job {
            Job::Keys => {
                m.keys().await;
                m.asking.store(false, Ordering::Relaxed);
            }
            Job::Mirror(path, mode) => {
                if let Err(e) = m.mirror_one(&router, &path, mode).await {
                    tracing::warn!(path, error = %format!("{e:#}"), "mirroring failed; it is served from here until the next miss");
                }
                m.pending.lock().unwrap_or_else(|e| e.into_inner()).remove(&path);
            }
        }
    }
}

async fn retry<T, F, Fut>(mut f: F) -> anyhow::Result<T>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = anyhow::Result<T>>,
{
    let mut wait = Duration::from_secs(2);
    for attempt in 1.. {
        match f().await {
            Ok(v) => return Ok(v),
            Err(e) if attempt >= 3 => return Err(e),
            Err(e) => {
                tracing::debug!(attempt, error = %format!("{e:#}"), "retrying an upload call");
                tokio::time::sleep(wait).await;
                wait *= 4;
            }
        }
    }
    unreachable!()
}

/// Is the bucket still certain to hold what was uploaded under this grant?
///
/// Its lifecycle rule expires an object `lifetime` after upload; a row stops
/// vouching a day before that. A lifetime too short to leave anything is never
/// fresh, and every miss uploads again — slow and correct.
fn fresh(row: &Row, grant: &Grant, now: i64) -> bool {
    row.target == grant.target() && row.uploaded_at + grant.lifetime.as_secs() as i64 - LIFECYCLE_SLACK > now
}

/// The whole object's length — the body's for a `200`, `Content-Range`'s total for a
/// `206`. Unknown for a compressed body, which nothing worth copying gets.
fn whole_len(resp: &Response) -> Option<u64> {
    let h = resp.headers();
    if resp.status() == StatusCode::PARTIAL_CONTENT {
        return header(h, CONTENT_RANGE).rsplit('/').next().and_then(|t| t.trim().parse().ok());
    }
    header(h, CONTENT_LENGTH).parse().ok().or_else(|| resp.body().size_hint().exact())
}

fn immutable(h: &HeaderMap) -> bool {
    header(h, CACHE_CONTROL).to_ascii_lowercase().contains("immutable")
}

/// Plain enough that the key the edge builds from the path it was asked for is,
/// byte for byte, the one uploaded.
fn signable(path: &str) -> bool {
    path.len() <= 1024
        && path.starts_with('/')
        && !path.contains("//")
        && path.bytes().all(|b| b.is_ascii_alphanumeric() || b"-._~/".contains(&b))
        && !path.split('/').any(|s| s == "." || s == "..")
}

fn header(h: &HeaderMap, name: HeaderName) -> String {
    h.get(name).and_then(|v| v.to_str().ok()).unwrap_or("").to_string()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grant() -> Grant {
        Grant {
            handle: "ana".into(),
            bucket: "cache-1".into(),
            region: "ap-beijing".into(),
            prefix: "cache/ana/".into(),
            paths: vec!["/api/media/".into(), "/api/attachments/".into(), "/views/".into()],
            write: cos::WriteKey {
                secret_id: "id".into(),
                secret_key: "k".into(),
                token: "t".into(),
                expires_at: chrono::Utc::now(),
            },
            sign_key: Arc::from(&b"k"[..]),
            lifetime: Duration::from_secs(30 * 86_400),
        }
    }

    const P: &str = "/api/media/file/2026-09-22/14/03-22.jpg";
    const NOW: i64 = 1_800_000_000;

    #[test]
    fn the_key_is_the_request_path_under_the_handle() {
        assert_eq!(grant().key(P), "cache/ana/api/media/file/2026-09-22/14/03-22.jpg");
    }

    #[test]
    fn the_whole_length_is_read_off_a_range() {
        let r = axum::http::Response::builder()
            .status(206)
            .header(CONTENT_RANGE, "bytes 0-1/481633429")
            .body(Body::from("01"))
            .unwrap();
        assert_eq!(whole_len(&r), Some(481_633_429));
        let r = axum::http::Response::builder().status(200).body(Body::from("0123")).unwrap();
        assert_eq!(whole_len(&r), Some(4));
    }

    /// The bodiless answer the edge reads as "serve it from the bucket, under this tag".
    #[test]
    fn the_bucket_answer_carries_the_version_and_no_body() {
        let r = in_bucket(HeaderValue::from_static("W/\"a-1\""));
        assert_eq!(r.status(), StatusCode::NOT_MODIFIED);
        assert_eq!(r.headers()[X_HI_CACHE], "bucket");
        assert_eq!(r.headers()[ETAG], "W/\"a-1\"");
        assert_eq!(r.body().size_hint().exact(), Some(0));
    }

    #[test]
    fn only_fronted_paths_are_mirrored() {
        let g = grant();
        assert!(g.fronts(P));
        assert!(g.fronts("/api/attachments/ab12/preview"));
        assert!(g.fronts("/views/badminton/clips/a.mp4"));
        assert!(!g.fronts("/assets/index.js"));
    }

    /// A row vouches until a day before the lifecycle drops its object, and only
    /// for the bucket and prefix it went up under.
    #[test]
    fn a_row_is_fresh_for_its_own_target_until_a_day_before_the_lifecycle() {
        let g = grant();
        let row = |at| Row { target: "cache-1/cache/ana/".into(), uploaded_at: at, etag: String::new() };
        assert!(fresh(&row(NOW), &g, NOW));
        assert!(fresh(&row(NOW - 29 * 86_400 + 60), &g, NOW));
        assert!(!fresh(&row(NOW - 29 * 86_400), &g, NOW));
        let other = Row { target: "cache-1/cache/bob/".into(), uploaded_at: NOW, etag: String::new() };
        assert!(!fresh(&other, &g, NOW));
    }

    #[test]
    fn immutable_is_read_off_cache_control() {
        let mut h = HeaderMap::new();
        assert!(!immutable(&h));
        h.insert(CACHE_CONTROL, "private, max-age=31536000, Immutable".parse().unwrap());
        assert!(immutable(&h));
        h.insert(CACHE_CONTROL, "private, max-age=31536000".parse().unwrap());
        assert!(!immutable(&h));
    }

    #[test]
    fn only_plain_paths_are_mirrored() {
        assert!(signable(P));
        assert!(!signable("/api/media/drive/照片.jpg"));
        assert!(!signable("/api/media/a%20b.jpg"));
        assert!(!signable("/api/media/../x"));
        assert!(!signable("api/media/x"));
        assert!(!signable("/a//b"));
    }

    /// Records every call, and fails the part numbered `fail_part` every time.
    #[derive(Default)]
    struct Fake {
        calls: Mutex<Vec<String>>,
        fail_part: Option<u32>,
    }

    impl Fake {
        fn calls(&self) -> Vec<String> {
            self.calls.lock().unwrap().clone()
        }
        fn log(&self, s: String) {
            self.calls.lock().unwrap().push(s);
        }
    }

    impl Bucket for Fake {
        async fn put(&self, key: &str, _: &cos::Meta<'_>, body: Bytes) -> anyhow::Result<()> {
            self.log(format!("put {key} {}", body.len()));
            Ok(())
        }
        async fn initiate(&self, key: &str, _: &cos::Meta<'_>) -> anyhow::Result<String> {
            self.log(format!("initiate {key}"));
            Ok("U".into())
        }
        async fn part(&self, _: &str, id: &str, n: u32, body: Bytes) -> anyhow::Result<String> {
            self.log(format!("part {id} {n} {}", body.len()));
            anyhow::ensure!(self.fail_part != Some(n), "refused");
            Ok(format!("\"e{n}\""))
        }
        async fn complete(&self, _: &str, id: &str, parts: &[(u32, String)]) -> anyhow::Result<()> {
            let etags: Vec<&str> = parts.iter().map(|(_, e)| e.as_str()).collect();
            self.log(format!("complete {id} {}", etags.join(",")));
            Ok(())
        }
        async fn abort(&self, _: &str, id: &str) {
            self.log(format!("abort {id}"));
        }
    }

    const META: cos::Meta<'static> = cos::Meta { content_type: "video/mp4", cache_control: "immutable" };

    /// A body in chunks of `chunk` bytes, `len` in all.
    fn body(len: usize, chunk: usize) -> Body {
        let chunks: Vec<Result<Bytes, std::io::Error>> =
            (0..len).step_by(chunk).map(|at| Ok(Bytes::from(vec![7u8; chunk.min(len - at)]))).collect();
        Body::from_stream(futures::stream::iter(chunks))
    }

    #[tokio::test]
    async fn a_small_object_goes_up_in_one_request() {
        let f = Fake::default();
        assert_eq!(upload(&f, "ana/x", &META, body(1000, 300)).await.unwrap(), 1000);
        assert_eq!(f.calls(), ["put ana/x 1000"]);
    }

    /// Parts fill across chunk boundaries, and the tail that is left when the body
    /// ends goes as a last, short part.
    #[tokio::test]
    async fn a_large_object_goes_up_in_parts() {
        let f = Fake::default();
        let len = 2 * PART + PART / 2;
        assert_eq!(upload(&f, "ana/v", &META, body(len, 3_000_000)).await.unwrap(), len as u64);
        assert_eq!(
            f.calls(),
            [
                "initiate ana/v".to_string(),
                format!("part U 1 {PART}"),
                format!("part U 2 {PART}"),
                format!("part U 3 {}", PART / 2),
                "complete U \"e1\",\"e2\",\"e3\"".to_string(),
            ]
        );
    }

    /// Exactly one part's worth is a one-part upload, not a part and an empty tail.
    #[tokio::test]
    async fn an_object_of_exactly_one_part_leaves_no_empty_tail() {
        let f = Fake::default();
        upload(&f, "ana/v", &META, body(PART, PART)).await.unwrap();
        assert_eq!(
            f.calls(),
            ["initiate ana/v".to_string(), format!("part U 1 {PART}"), "complete U \"e1\"".to_string()]
        );
    }

    /// A part that keeps failing is retried, then the upload is abandoned and
    /// aborted — its parts must not sit in the bucket being paid for.
    #[tokio::test(start_paused = true)]
    async fn a_part_that_keeps_failing_aborts_the_upload() {
        let f = Fake { fail_part: Some(2), ..Default::default() };
        assert!(upload(&f, "ana/v", &META, body(3 * PART, PART)).await.is_err());
        let calls = f.calls();
        assert_eq!(calls.iter().filter(|c| c.starts_with("part U 2")).count(), 3, "{calls:?}");
        assert_eq!(calls.last().map(String::as_str), Some("abort U"));
        assert!(!calls.iter().any(|c| c.starts_with("complete")));
    }
}
