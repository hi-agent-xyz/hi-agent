//! Mirroring — immutable bytes leave this machine ahead of being asked for, and a
//! request that came through the tunnel for one is answered with a redirect to the
//! community's cache instead of the bytes.
//!
//! See `docs/arch/topology.md` § *Content*. The short of it: the relay is an
//! 11 Mbps box, a conversation fits through it and a photograph does not, so
//! content takes a second path — uploaded from here straight to a bucket, over an
//! uplink that is otherwise idle, and fetched by the app from the edge in front of
//! that bucket.
//!
//! ## What may be mirrored: the response already says
//!
//! **A response whose `Cache-Control` says `immutable` may be mirrored, and nothing
//! else may.** Not a list of routes: the one judgment a person has to make — *do
//! these bytes ever change?* — is the one that header already asks, and a route
//! that forgets to say so costs acceleration and nothing else.
//!
//! Two narrower conditions ride on top, both about *serving* from another origin
//! rather than about the bytes:
//!
//! - **The content type is one that cannot resolve anything against its own URL**
//!   — pictures, sound, video, fonts, PDF. The redirect changes the URL a response
//!   is read from, and the signature is in its query, so a stylesheet's `url(./x)`
//!   or a module's `import "./x.js"` would resolve beside it at the edge *without*
//!   a signature and be refused. Script, style and markup stay here.
//! - **The path is plain ASCII**, so the path the edge hashes is unambiguously the
//!   path that was signed.
//!
//! ## Only requests that came through the tunnel are redirected
//!
//! The cache exists to take content off the tunnel. A request that arrived on a
//! public bind or over the home network has no tunnel to relieve, and a phone on
//! the same Wi-Fi would be sent from a local link to a remote edge. So the
//! redirect needs the [`Relayed`] marker the tunnel puts on what it routes in;
//! loopback and every other listener are served as they always were.
//!
//! ## The two branches
//!
//! For a relayed `GET`, the route runs as it always did, and then:
//!
//! - mirrored → the body is dropped and a `302` to a signed edge URL goes back;
//! - not yet → the bytes go back, as today, and the path is queued for upload.
//!
//! **The second branch is a degradation, not an error**: an object that is not
//! mirrored is exactly as slow as it was before any of this, so there is no flag
//! day and nothing to migrate. A core with no handle, or a community with no
//! cache configured, simply takes it forever.
//!
//! **Running the route first is what makes the redirect safe.** It costs a `stat`
//! and an open, and it means the core has checked, at the moment of redirecting,
//! that the object still exists, still calls itself immutable, and is still the
//! length that was uploaded. A file deleted here stops being redirected to at
//! once; one that faded to a keepsake stops claiming immutability and its row is
//! forgotten; and one whose bytes changed *while still claiming* immutability is
//! caught, logged by path, and never mirrored again.

mod cos;
mod credential;
mod edge;
mod record;

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, RwLock};
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::extract::Request;
use axum::http::header::{CACHE_CONTROL, CONTENT_LENGTH, CONTENT_RANGE, CONTENT_TYPE, LOCATION};
use axum::http::{HeaderMap, HeaderName, HeaderValue, Method, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse as _, Response};
use bytes::{Bytes, BytesMut};
use futures::StreamExt as _;
use axum::body::HttpBody as _;
use tokio::sync::mpsc;
use tower::ServiceExt as _;

use crate::foundation::surfaces::Acceptor;
use crate::foundation::tunnel::{self, Relayed};
use credential::{Answer, Grant};
use record::{Records, Row};

/// The size above which an upload goes in parts. Each part is retried on its own,
/// so a home connection that drops mid-video loses one part, not the video.
///
/// **That is the whole of the resumability today.** An upload cut off by a restart,
/// or abandoned after a part failed three times, starts again from nothing the next
/// time the object is asked for; the parts it left are removed by the bucket's
/// rule for incomplete uploads. Resuming across a restart would mean keeping the
/// upload id and part list in the record, which nothing does yet.
const PART: usize = 8 * 1024 * 1024;

/// Renew the write key this long before it runs out, so a part is never sent on
/// a key that expires while it is in flight.
const WRITE_KEY_MARGIN: chrono::Duration = chrono::Duration::minutes(5);

/// How long to stop asking after the community declined, or could not be reached.
/// Only demand asks at all — a queued upload, or a request for something already
/// mirrored — so this bounds how often demand turns into a call, not a timer.
const DECLINED_FOR: Duration = Duration::from_secs(15 * 60);
const UNREACHABLE_FOR: Duration = Duration::from_secs(5 * 60);

static MIRROR: OnceLock<Mirror> = OnceLock::new();

struct Mirror {
    data_dir: PathBuf,
    records: Records,
    /// The keys, once the community has handed them over. Read on every relayed
    /// request for something mirrorable, written about once an hour.
    grant: RwLock<Option<Arc<Grant>>>,
    queue: mpsc::UnboundedSender<String>,
    /// Paths queued and not yet done, so a page of forty pictures asked for twice
    /// queues forty uploads, not eighty.
    pending: Mutex<HashSet<String>>,
    /// Unix seconds before which asking again is pointless.
    quiet_until: AtomicI64,
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
        quiet_until: AtomicI64::new(0),
    };
    if MIRROR.set(mirror).is_err() {
        return;
    }
    tokio::spawn(work(router, rx));
}

/// Queue `path` — a request path this core serves — to be mirrored.
///
/// What a write calls, right after the bytes are durable: **the trigger is the
/// core writing the file, not someone asking for it.** A photo that arrives is
/// uploaded while nobody is looking, which is the whole advantage over a cache in
/// front of the tunnel — that could only ever speed up the second look, and in a
/// life record the first look is the common case.
///
/// Free when mirroring is off, and harmless for a path that turns out not to be
/// mirrorable: the uploader asks the route and drops what it will not vouch for.
pub fn enqueue(path: &str) {
    let Some(m) = MIRROR.get() else { return };
    if chrono::Utc::now().timestamp() < m.quiet_until.load(Ordering::Relaxed) || !signable(path) {
        return;
    }
    let fresh = m.pending.lock().unwrap_or_else(|e| e.into_inner()).insert(path.to_string());
    if fresh {
        let _ = m.queue.send(path.to_string());
    }
}

/// The layer: between the gate and the routes, so nothing unauthorized is ever
/// redirected and the route has already answered by the time this decides.
pub async fn layer(req: Request, next: Next) -> Response {
    let relayed = req.extensions().get::<Relayed>().is_some();
    let Some(m) = MIRROR.get().filter(|_| relayed && req.method() == Method::GET) else {
        return next.run(req).await;
    };
    let path = req.uri().path().to_string();
    let resp = next.run(req).await;
    m.answer(&path, resp)
}

impl Mirror {
    fn answer(&self, path: &str, resp: Response) -> Response {
        let seen = Seen::of(&resp);
        if !seen.worth_a_look() {
            return resp;
        }
        let row = self.records.get(path);
        let grant = self.grant.read().unwrap_or_else(|e| e.into_inner()).clone();
        let now = chrono::Utc::now().timestamp();
        // **Reads re-ask too, about once an hour.** The grant is otherwise renewed only
        // for an upload, and a core that is only being looked at would go on signing
        // for a bucket the community has replaced, or a handle it has renamed, or
        // after the community stopped granting at all. Once the write key has run out,
        // a request for anything mirrorable queues its path; the uploader renews on
        // the way to finding the row fresh, and this request is still redirected on
        // the keys in hand, so asking costs nobody a wait.
        if grant.as_ref().is_some_and(|g| g.write.expires_at.timestamp() < now) && row.is_some() {
            enqueue(path);
        }
        match decide(path, &seen, row.as_ref(), grant.as_deref(), now) {
            Verdict::Serve => resp,
            Verdict::Enqueue => {
                enqueue(path);
                resp
            }
            Verdict::Forget => {
                self.records.forget(path);
                resp
            }
            Verdict::Changed => {
                // The one failure this design can have, caught: a route that says
                // its bytes never change, whose bytes changed. The cure is at the
                // route, and this names it.
                tracing::warn!(
                    path,
                    uploaded = ?row.map(|r| r.len),
                    now = ?seen.len,
                    "a response marked immutable changed length; never mirroring it again"
                );
                self.records.changed(path);
                resp
            }
            Verdict::Redirect { url, max_age } => {
                let mut r = StatusCode::FOUND.into_response();
                if let Ok(v) = HeaderValue::from_str(&url) {
                    r.headers_mut().insert(LOCATION, v);
                } else {
                    return resp;
                }
                // Browser-cacheable, so a redirect's round trip is paid once per
                // object per device, not once per render. `private`: it was issued
                // because a credential checked out.
                if let Ok(v) = HeaderValue::from_str(&format!("private, max-age={max_age}")) {
                    r.headers_mut().insert(CACHE_CONTROL, v);
                }
                r
            }
        }
    }

    /// The keys for an upload: the ones in hand if they are for this handle and
    /// have a while left to run, otherwise fresh ones. `None` when there is nothing
    /// to upload for — no handle served, or the community declined.
    async fn grant_for_upload(&self) -> Option<Arc<Grant>> {
        let now = chrono::Utc::now();
        if now.timestamp() < self.quiet_until.load(Ordering::Relaxed) {
            return None;
        }
        // Nobody can reach this core by name, so nothing will ever be redirected.
        let Some(handle) = tunnel::chosen(&self.data_dir).filter(|_| tunnel::on(&self.data_dir))
        else {
            self.quiet(DECLINED_FOR);
            return None;
        };
        if let Some(g) = self.grant.read().unwrap_or_else(|e| e.into_inner()).clone() {
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
                // Declining to mint is how a core's access is withdrawn, so the read
                // key goes too: no redirect is issued on the strength of old keys.
                tracing::info!(%handle, %why, "the community declined a cache grant; serving every object from here");
                *self.grant.write().unwrap_or_else(|e| e.into_inner()) = None;
                self.quiet(DECLINED_FOR);
                None
            }
            Err(e) => {
                // Unreachable is not declined: keep signing redirects for what is
                // already up there, and try again shortly.
                tracing::warn!(%handle, error = %format!("{e:#}"), "could not get a cache grant");
                self.quiet(UNREACHABLE_FOR);
                None
            }
        }
    }

    fn quiet(&self, d: Duration) {
        let until = chrono::Utc::now().timestamp() + d.as_secs() as i64;
        self.quiet_until.store(until, Ordering::Relaxed);
    }

    /// Mirror one path, if it wants mirroring and the keys are to hand.
    async fn mirror_one(&self, router: &Router, path: &str) -> anyhow::Result<()> {
        let Some(grant) = self.grant_for_upload().await else { return Ok(()) };
        let now = chrono::Utc::now().timestamp();
        if let Some(row) = self.records.get(path) {
            if row.changed || (row.target == grant.target() && fresh(&row, &grant, now)) {
                return Ok(());
            }
        }

        let mut req = Request::get(path).body(Body::empty())?;
        req.extensions_mut().insert(Acceptor::Loopback);
        let resp = router.clone().oneshot(req).await.unwrap_or_else(|e| match e {});
        let seen = Seen::of(&resp);
        if seen.status != StatusCode::OK || !seen.immutable || !seen.mirrorable {
            return Ok(());
        }
        let content_type = header(&resp, CONTENT_TYPE);
        let cache_control = header(&resp, CACHE_CONTROL);
        let meta = cos::Meta { content_type: &content_type, cache_control: &cache_control };

        let key = grant.key(path);
        let target = grant.target();
        let bucket = Renewing { mirror: self, current: tokio::sync::Mutex::new(None), grant };
        let total = upload(&bucket, &key, &meta, resp.into_body()).await?;
        self.records.uploaded(path, &target, total, chrono::Utc::now().timestamp());
        tracing::info!(path, bytes = total, "mirrored");
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
                .grant_for_upload()
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

/// The uploader: one object at a time, in the order they were written.
///
/// **One at a time is the whole of "low priority" today.** The design asks for
/// uploads to run when the uplink is otherwise idle, and nothing here measures
/// that yet: a large upload does share the uplink with the tunnel while it runs.
/// Sequential bounds that to one stream; watching the tunnel's own traffic is what
/// would finish the job.
async fn work(router: Router, mut rx: mpsc::UnboundedReceiver<String>) {
    let Some(m) = MIRROR.get() else { return };
    while let Some(path) = rx.recv().await {
        if let Err(e) = m.mirror_one(&router, &path).await {
            tracing::warn!(path, error = %format!("{e:#}"), "mirroring failed; it is served from here until asked for again");
        }
        m.pending.lock().unwrap_or_else(|e| e.into_inner()).remove(&path);
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

/// What a response says about itself, as far as mirroring cares.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Seen {
    status: StatusCode,
    immutable: bool,
    mirrorable: bool,
    /// The whole object's length — the body's for a `200`, `Content-Range`'s
    /// total for a `206`. Unknown for a compressed body, which none of the
    /// mirrorable types get.
    len: Option<u64>,
}

impl Seen {
    fn of(resp: &Response) -> Self {
        let h = resp.headers();
        let len = if resp.status() == StatusCode::PARTIAL_CONTENT {
            text(h, CONTENT_RANGE).rsplit('/').next().and_then(|t| t.trim().parse().ok())
        } else {
            text(h, CONTENT_LENGTH).parse().ok().or_else(|| resp.body().size_hint().exact())
        };
        Seen {
            status: resp.status(),
            immutable: text(h, CACHE_CONTROL).to_ascii_lowercase().contains("immutable"),
            mirrorable: mirrorable(text(h, CONTENT_TYPE)),
            len,
        }
    }

    /// Everything else — every API call, every page — is passed straight through
    /// without touching the record.
    fn worth_a_look(&self) -> bool {
        matches!(self.status, StatusCode::OK | StatusCode::PARTIAL_CONTENT) && self.mirrorable
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Verdict {
    Serve,
    Enqueue,
    Forget,
    Changed,
    Redirect { url: String, max_age: u64 },
}

fn decide(path: &str, seen: &Seen, row: Option<&Row>, grant: Option<&Grant>, now: i64) -> Verdict {
    if !seen.worth_a_look() {
        return Verdict::Serve;
    }
    if !seen.immutable || !signable(path) {
        // It was mirrored and no longer claims to be immutable — a picture that
        // faded to its keepsake. Stop naming the copy.
        return match row {
            Some(r) if !r.changed => Verdict::Forget,
            _ => Verdict::Serve,
        };
    }
    let Some(row) = row else { return Verdict::Enqueue };
    if row.changed {
        return Verdict::Serve;
    }
    if seen.len.is_some_and(|len| len != row.len) {
        return Verdict::Changed;
    }
    // Mirrored, but the keys are not in hand yet — after a restart, typically.
    // Queueing is what fetches them; the uploader will find the row and stop there.
    let Some(grant) = grant else { return Verdict::Enqueue };
    if row.target != grant.target() || !fresh(row, grant, now) {
        return Verdict::Enqueue;
    }
    Verdict::Redirect {
        url: grant.read.signed_url(&format!("/{}", grant.key(path)), now),
        max_age: grant.read.redirect_max_age(),
    }
}

/// Is the bucket still certain to hold what was uploaded, for as long as a redirect
/// issued now can be followed?
///
/// Its lifecycle rule expires an object `lifetime` after upload. A redirect issued
/// now names a URL that works for up to one signature validity, so a row stops
/// vouching that long before the lifecycle — and a day more, for however the
/// bucket rounds its expiry to a day. A lifetime too short to leave anything is
/// never fresh: every object is served from here, which is slow and correct.
fn fresh(row: &Row, grant: &Grant, now: i64) -> bool {
    let lifetime = grant.lifetime.as_secs() as i64;
    let margin = grant.read.valid.as_secs() as i64 + 86_400;
    row.uploaded_at + lifetime - margin > now
}

/// Pictures, sound, video, fonts, PDF: bytes that cannot resolve anything against
/// the URL they were read from. See the module docs for why that is the line.
fn mirrorable(content_type: &str) -> bool {
    let t = content_type.split(';').next().unwrap_or("").trim().to_ascii_lowercase();
    (t.starts_with("image/") && t != "image/svg+xml")
        || t.starts_with("video/")
        || t.starts_with("audio/")
        || t.starts_with("font/")
        || t == "application/pdf"
}

/// Plain enough that the path the edge hashes is, byte for byte, the one signed.
fn signable(path: &str) -> bool {
    path.len() <= 1024
        && path.starts_with('/')
        && !path.contains("//")
        && path.bytes().all(|b| b.is_ascii_alphanumeric() || b"-._~/".contains(&b))
        && !path.split('/').any(|s| s == "." || s == "..")
}

fn text(h: &HeaderMap, name: HeaderName) -> &str {
    h.get(name).and_then(|v| v.to_str().ok()).unwrap_or("")
}

fn header(resp: &Response, name: HeaderName) -> String {
    text(resp.headers(), name).to_string()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seen(status: u16, immutable: bool, len: Option<u64>) -> Seen {
        Seen { status: StatusCode::from_u16(status).unwrap(), immutable, mirrorable: true, len }
    }

    fn grant() -> Grant {
        Grant {
            handle: "ana".into(),
            bucket: "cache-1".into(),
            region: "ap-beijing".into(),
            prefix: "ana/".into(),
            write: cos::WriteKey {
                secret_id: "id".into(),
                secret_key: "k".into(),
                token: "t".into(),
                expires_at: chrono::Utc::now(),
            },
            read: edge::ReadKey {
                base_url: "https://media.example".into(),
                param: "auth_key".into(),
                key: "k".into(),
                valid: Duration::from_secs(86_400),
            },
            lifetime: Duration::from_secs(30 * 86_400),
        }
    }

    fn row(len: u64, uploaded_at: i64) -> Row {
        Row { target: "cache-1/ana/".into(), len, uploaded_at, changed: false }
    }

    const P: &str = "/api/media/file/2026-09-22/14/03-22.jpg";
    const NOW: i64 = 1_800_000_000;

    #[test]
    fn a_mirrored_object_is_redirected_to_its_own_key() {
        let g = grant();
        match decide(P, &seen(200, true, Some(10)), Some(&row(10, NOW)), Some(&g), NOW) {
            Verdict::Redirect { url, max_age } => {
                assert!(url.starts_with(
                    "https://media.example/ana/api/media/file/2026-09-22/14/03-22.jpg?auth_key="
                ));
                assert_eq!(max_age, 43_200);
            }
            v => panic!("{v:?}"),
        }
    }

    /// A seek is a `206`; its length is the whole object's, from `Content-Range`.
    #[test]
    fn a_range_request_is_redirected_too() {
        let g = grant();
        let v = decide(P, &seen(206, true, Some(10)), Some(&row(10, NOW)), Some(&g), NOW);
        assert!(matches!(v, Verdict::Redirect { .. }));
    }

    #[test]
    fn nothing_mirrored_yet_serves_the_bytes_and_queues_the_upload() {
        assert_eq!(decide(P, &seen(200, true, Some(10)), None, Some(&grant()), NOW), Verdict::Enqueue);
    }

    /// After a restart the record is there and the keys are not: serve, and let
    /// queueing fetch them.
    #[test]
    fn a_mirrored_object_without_keys_in_hand_is_served_here() {
        assert_eq!(decide(P, &seen(200, true, Some(10)), Some(&row(10, NOW)), None, NOW), Verdict::Enqueue);
    }

    #[test]
    fn the_route_decides_what_is_mirrorable_not_the_path() {
        assert_eq!(decide(P, &seen(200, false, Some(10)), None, Some(&grant()), NOW), Verdict::Serve);
        let mut s = seen(200, true, Some(10));
        s.mirrorable = false;
        assert_eq!(decide(P, &s, None, Some(&grant()), NOW), Verdict::Serve);
        assert_eq!(decide(P, &seen(404, true, None), Some(&row(10, NOW)), Some(&grant()), NOW), Verdict::Serve);
    }

    /// A picture faded to its keepsake no longer says immutable; the copy of the
    /// original must stop being named at once.
    #[test]
    fn a_path_that_stops_claiming_immutable_is_forgotten() {
        assert_eq!(
            decide(P, &seen(200, false, Some(3)), Some(&row(10, NOW)), Some(&grant()), NOW),
            Verdict::Forget
        );
    }

    #[test]
    fn a_length_that_changed_under_an_immutable_label_is_caught() {
        assert_eq!(
            decide(P, &seen(200, true, Some(11)), Some(&row(10, NOW)), Some(&grant()), NOW),
            Verdict::Changed
        );
        let mut r = row(10, NOW);
        r.changed = true;
        assert_eq!(decide(P, &seen(200, true, Some(10)), Some(&r), Some(&grant()), NOW), Verdict::Serve);
    }

    /// A renamed handle or a new bucket reads as not mirrored; so does a row the
    /// bucket's lifecycle is about to catch up with.
    #[test]
    fn a_row_for_another_target_or_near_its_expiry_is_uploaded_again() {
        let mut other = row(10, NOW);
        other.target = "cache-1/bob/".into();
        assert_eq!(decide(P, &seen(200, true, Some(10)), Some(&other), Some(&grant()), NOW), Verdict::Enqueue);
        // 30-day lifecycle, 1-day signatures: a row vouches for 28 days.
        let old = row(10, NOW - 28 * 86_400);
        assert_eq!(decide(P, &seen(200, true, Some(10)), Some(&old), Some(&grant()), NOW), Verdict::Enqueue);
        let young = row(10, NOW - 28 * 86_400 + 60);
        assert!(matches!(
            decide(P, &seen(200, true, Some(10)), Some(&young), Some(&grant()), NOW),
            Verdict::Redirect { .. }
        ));
    }

    /// A redirect is followable for a whole signature validity after it is issued, so
    /// a lifecycle that leaves no room for one never redirects at all.
    #[test]
    fn a_lifetime_shorter_than_a_signature_redirects_nothing() {
        let mut g = grant();
        g.lifetime = Duration::from_secs(2 * 86_400);
        assert_eq!(decide(P, &seen(200, true, Some(10)), Some(&row(10, NOW)), Some(&g), NOW), Verdict::Enqueue);
    }

    #[test]
    fn only_plain_paths_are_signed() {
        assert!(signable(P));
        assert!(signable("/views/_shots/0a1b.png"));
        assert!(!signable("/api/media/drive/照片.jpg"));
        assert!(!signable("/api/media/a%20b.jpg"));
        assert!(!signable("/api/media/../x"));
        assert!(!signable("api/media/x"));
        assert!(!signable("/a//b"));
    }

    #[test]
    fn only_bytes_that_resolve_nothing_are_mirrorable() {
        for t in ["image/jpeg", "video/mp4", "audio/mpeg", "font/woff2", "application/pdf", "image/png; x=y"] {
            assert!(mirrorable(t), "{t}");
        }
        for t in ["text/javascript", "text/css", "text/html", "image/svg+xml", "application/json", ""] {
            assert!(!mirrorable(t), "{t}");
        }
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

    #[test]
    fn seen_reads_the_whole_length_off_a_range() {
        let r = axum::http::Response::builder()
            .status(206)
            .header(CONTENT_RANGE, "bytes 0-1/481633429")
            .header(CONTENT_TYPE, "video/mp4")
            .header(CACHE_CONTROL, "private, max-age=31536000, immutable")
            .body(Body::from("01"))
            .unwrap();
        let s = Seen::of(&r);
        assert_eq!(s.len, Some(481_633_429));
        assert!(s.immutable && s.mirrorable);
    }
}
