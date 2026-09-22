//! Attachments — what was put in front of the person, copied at the moment it was
//! (`docs/arch/showing.md`).
//!
//! **A picture reaches the person carried by the sentence that says what it shows**, and
//! this is the half of that which is about bytes. A mind names a path; this copies it into
//! `data/attachments/`, addresses it by the SHA-256 of what was copied, reads what it is by
//! decoding it, and draws the one small picture every surface shows for it. The mind gets an
//! id back and never types a hash, a URL or a type.
//!
//! **Copied, not referenced, because a line says what was true when it was written.** The
//! worker that attached `pose_899.png` overwrites it on its next iteration, and `/tmp` does
//! not outlive the session that wrote there — the court-calibration task's first shift left
//! all of its work in `/tmp/cc`. A reference would silently change what the evidence shows;
//! a copy cannot. `tokio::fs::copy` is `std::fs::copy`, which clones on APFS (and
//! `copy_file_range` reflinks on btrfs/XFS), so the copy is free until the original changes.
//!
//! **Addressed by content, so `immutable` is true by construction.** The same bytes attached
//! twice are one object, and the route serving them can promise they never change — which
//! is what lets a browser keep them forever and what marks them mirrorable at all
//! (`docs/arch/topology.md` § *Content*).
//!
//! **The host's pen.** Nothing else writes under `data/attachments/`; it is written at a seam,
//! the way the log is, and kept the way text is kept. `derived/` beside the objects is a
//! disposable cache — every file in it can be made again from its object.
//!
//! The word is the conversation's — [`crate::foundation::server::Attachment`] is a file on a
//! message — and not `body::attachments`, which counts which windows and speakers are
//! subscribed to the out-channels.
//!
//! What this accepts **today** is pictures and clips. A recording or a document is refused
//! with a sentence that says so, rather than accepted and drawn as nothing: showing.md's
//! later phases add them, and until then the refusal is the honest answer.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{LazyLock, Mutex};
use std::time::Instant;

use anyhow::{Context, bail};
use image::{DynamicImage, ImageDecoder, ImageFormat};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::process::Command;

/// How an attachment is named everywhere a mind or a surface names one: `att:<id>`.
pub const PREFIX: &str = "att:";

/// Hex digits of the SHA-256 an id keeps. 64 bits: a collision among a person's lifetime
/// of attachments is not a thing that happens, and an id short enough to sit in a line of
/// a mind's window is one it will actually copy right.
const ID_LEN: usize = 16;

/// The largest file this copies. A starting value (showing.md § *Open*): big enough for any
/// clip a result is, and a refusal rather than an hour of hashing for the rare thing past it.
pub const MAX_BYTES: u64 = 2 * 1024 * 1024 * 1024;

/// The derivation every surface draws an attachment by. Versioned because its URL is
/// served `immutable`: a change to how previews are made is a new spec, and so a new URL,
/// never new bytes at an old one.
pub const PREVIEW_SPEC: &str = "preview.v1";

/// A preview fits inside this box and is never larger than its source. 960×540 is the
/// tile rule every picture on Home already keeps (`docs/arch/stage.md` § *The frame is a
/// surface*): a 240×135 box at 2× zoom on a 2× screen.
const PREVIEW_W: u32 = 960;
const PREVIEW_H: u32 = 540;

/// JPEG quality for a preview. A tile is looked at, not edited; 82 is where text in a
/// screenshot stops showing ringing at the tile's size.
const PREVIEW_QUALITY: u8 = 82;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Picture,
    Clip,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Picture => "picture",
            Self::Clip => "clip",
        }
    }
}

/// What an attachment *is* — facts about the bytes and nothing else. Who attached it, to
/// what, and why are the line's, never the object's: one picture on three lines is three
/// claims about one thing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Probe {
    pub kind: Kind,
    pub ext: String,
    pub bytes: u64,
    pub width: u32,
    pub height: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fps: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub codec: Option<String>,
    pub sha256: String,
}

impl Probe {
    /// The type this is served as — the same table every file route here reads.
    pub fn mime(&self) -> &'static str {
        crate::mind::memory::media::content_type(&format!("x.{}", self.ext))
    }

    /// What it is, for a reader: `picture 1920×1080`, `clip 1920×1080 · 0:30`. The frame rate
    /// and the note about a copy are machinery, so they stay in [`Probe::describe`] — a page
    /// somebody outside opens carries this.
    pub fn what(&self) -> String {
        let mut out = format!("{} {}×{}", self.kind.as_str(), self.width, self.height);
        if let Some(ms) = self.duration_ms {
            out.push_str(&format!(" · {}", clock(ms)));
        }
        out
    }

    /// What a mind is told it attached, in one line: `picture 1920×1080`, `clip 1920×1080 · 0:30 · 30 fps`.
    pub fn describe(&self) -> String {
        let mut out = self.what();
        if let Some(fps) = self.fps {
            out.push_str(&format!(" · {} fps", trim_float(fps)));
        }
        if !self.playable() {
            out.push_str(" · a copy browsers can play is being made");
        }
        out
    }

    /// Whether a browser this product ships in can play the bytes as they are. A picture is
    /// only ever kept in a format one can show; a clip is playable when both its container and
    /// its video codec are. One that is not is played from its `proxy.v1` copy.
    pub fn playable(&self) -> bool {
        match self.kind {
            Kind::Picture => true,
            Kind::Clip => {
                matches!(self.ext.as_str(), "mp4" | "mov" | "webm")
                    && self.codec.as_deref().is_some_and(|c| BROWSER_CODECS.contains(&c))
            }
        }
    }
}

/// `0:30`, `12:04`, `1:02:09`.
pub fn clock(ms: u64) -> String {
    let s = (ms + 500) / 1000;
    let (h, m, s) = (s / 3600, (s / 60) % 60, s % 60);
    if h > 0 { format!("{h}:{m:02}:{s:02}") } else { format!("{m}:{s:02}") }
}

fn trim_float(v: f64) -> String {
    let s = format!("{v:.2}");
    s.trim_end_matches('0').trim_end_matches('.').to_owned()
}

/// One attachment, placed.
#[derive(Debug, Clone)]
pub struct Placed {
    /// The bare id; [`PREFIX`] is how it is written.
    pub id: String,
    pub probe: Probe,
}

/// Why a file was not attached. Each says what to do instead, because the reader is the
/// model that called the verb and will act on the words.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    Missing(String),
    NotAFile(String),
    Secret,
    TooLarge(String, u64),
    NotShowable(String, String),
    UnknownId(String),
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Missing(path) => write!(
                f,
                "`{path}` is not there — attach a file that exists: a path in your task's folder, or an absolute one"
            ),
            Self::NotAFile(path) => write!(f, "`{path}` is a directory, not a file"),
            Self::Secret => f.write_str(
                "that file is a stored secret, and a secret is never put in front of anyone",
            ),
            Self::TooLarge(path, bytes) => write!(
                f,
                "`{path}` is {} MB; an attachment is at most {} MB — cut a shorter clip or a smaller picture",
                bytes / (1024 * 1024),
                MAX_BYTES / (1024 * 1024)
            ),
            Self::NotShowable(path, why) => write!(f, "`{path}` {why}"),
            Self::UnknownId(id) => write!(f, "`{PREFIX}{id}` is no attachment this store holds"),
        }
    }
}

/// `att:<id>` or a bare id → the bare id, when it is the shape an id has.
pub fn parse_id(s: &str) -> Option<&str> {
    let id = s.trim().strip_prefix(PREFIX).unwrap_or(s.trim());
    (id.len() == ID_LEN && id.bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()))
        .then_some(id)
}

/// The URL an attachment's bytes are served at.
pub fn url(id: &str) -> String {
    format!("/api/attachments/{id}")
}

/// The URL its preview is served at. Carries the spec, so a new way of drawing previews is
/// a new URL rather than new bytes behind an `immutable` one.
pub fn preview_url(id: &str) -> String {
    format!("/api/attachments/{id}/{PREVIEW_SPEC}")
}

/// The module that puts an attachment on the stage. Versioned for the reason the preview is:
/// it is served `immutable`, so a change to what it says is a new name.
pub const STAGE_SPEC: &str = "stage.v1.mjs";

/// Where the stage module for `id` is served — what the stage mounts in place of a compiled
/// view (`docs/arch/showing.md` § *On the stage*).
pub fn stage_module_url(id: &str) -> String {
    format!("/api/attachments/{id}/{STAGE_SPEC}")
}

/// What a view is told about an attachment it embeds by id. Versioned and `immutable` for the
/// reason the preview is: the id fixes the probe, and the spec fixes what is said about it.
pub const ABOUT_SPEC: &str = "about.v1";

/// Where [`about`] is served for `id`.
pub fn about_url(id: &str) -> String {
    format!("/api/attachments/{id}/{ABOUT_SPEC}")
}

/// The face's own description of one attachment — the `AttachmentItem` every surface draws it
/// from (`appearance/web/src/core/attachments.tsx`), so the stage module that is handed it and
/// the `<Attachment>` a view embeds that fetches it cannot describe the same object two ways.
/// A clip's bytes are named by the route that says where its playable bytes are.
pub fn about(id: &str, probe: &Probe) -> serde_json::Value {
    serde_json::json!({
        "ref": format!("{PREFIX}{id}"),
        "kind": probe.kind.as_str(),
        "url": if probe.kind == Kind::Clip { playable_url(id) } else { url(id) },
        "preview": preview_url(id),
        "width": probe.width,
        "height": probe.height,
        "durationMs": probe.duration_ms,
    })
}

/// The bare id an `att:` ref names, or `None` for a view's ref. The stage and the trail carry an
/// attachment under the ref it is named by everywhere, so the one thing that tells the two
/// apart is the prefix.
pub fn ref_id(reference: &str) -> Option<&str> {
    let reference = reference.trim();
    reference.starts_with(PREFIX).then(|| parse_id(reference)).flatten()
}

/// The stage module for one attachment: three lines that draw the face's own `AttachmentStage`
/// (`appearance/web/src/core/attachments.tsx`) with what the probe knows.
///
/// **Not a view, and not compiled.** A view is built and can be wrong; showing a picture must
/// not depend on a builder having been right, nor on the view compiler or its cache, which is
/// why the conversation's own surface is bundled rather than compiled (`docs/arch/stage.md`
/// § 1). The component is the host's; this only names which attachment it draws, and its bare
/// imports resolve through the page's import map to the one shared instance, like every view's.
pub fn stage_module(id: &str, probe: &Probe) -> String {
    let item = about(id, probe);
    format!(
        "// One attachment on the stage (docs/arch/showing.md). Served by the host, never compiled.\n\
         import {{ jsx }} from \"react/jsx-runtime\";\n\
         import {{ AttachmentStage }} from \"@hi/core\";\n\
         const item = {item};\n\
         export default function Attachment() {{ return jsx(AttachmentStage, {{ item }}); }}\n"
    )
}

/// What `id` is, read without awaiting — for a caller building a label under a lock. The same
/// cache [`probe`] fills; a miss reads the probe's few hundred bytes off disk once.
pub fn probe_now(data_dir: &Path, id: &str) -> Option<Probe> {
    let id = parse_id(id)?;
    let path = probe_path(data_dir, id);
    if let Some(hit) = PROBES.lock().unwrap_or_else(|p| p.into_inner()).get(&path) {
        return Some(hit.clone());
    }
    let probe: Probe = serde_json::from_slice(&std::fs::read(&path).ok()?).ok()?;
    PROBES.lock().unwrap_or_else(|p| p.into_inner()).insert(path, probe.clone());
    Some(probe)
}

/// The name an attachment's card goes by where a view's would carry its own: what it is.
pub fn label(data_dir: &Path, id: &str) -> String {
    match probe_now(data_dir, id).map(|p| p.kind) {
        Some(Kind::Picture) => "Picture".to_owned(),
        Some(Kind::Clip) => "Clip".to_owned(),
        None => "Attachment".to_owned(),
    }
}

fn root(data_dir: &Path) -> PathBuf {
    data_dir.join("attachments")
}

fn shard(id: &str) -> &str {
    &id[..2]
}

fn object_path(data_dir: &Path, id: &str, ext: &str) -> PathBuf {
    root(data_dir).join("objects").join(shard(id)).join(format!("{id}.{ext}"))
}

fn probe_path(data_dir: &Path, id: &str) -> PathBuf {
    root(data_dir).join("objects").join(shard(id)).join(format!("{id}.json"))
}

fn preview_dir(data_dir: &Path, id: &str) -> PathBuf {
    root(data_dir).join("derived").join(PREVIEW_SPEC).join(shard(id))
}

/// Probes read so far, by the file they were read from. An object's probe never changes,
/// so a surface polling a ledger full of attachments reads each one off disk once.
static PROBES: LazyLock<Mutex<HashMap<PathBuf, Probe>>> = LazyLock::new(Default::default);

/// What `id` is, or `None` when this store holds no such attachment — a hand-typed id in a
/// record is ignored on read the way a `made` ref to a deleted view is.
pub async fn probe(data_dir: &Path, id: &str) -> Option<Probe> {
    let id = parse_id(id)?;
    let path = probe_path(data_dir, id);
    if let Some(hit) = PROBES.lock().unwrap_or_else(|p| p.into_inner()).get(&path) {
        return Some(hit.clone());
    }
    let raw = tokio::fs::read(&path).await.ok()?;
    let probe: Probe = serde_json::from_slice(&raw).ok()?;
    PROBES.lock().unwrap_or_else(|p| p.into_inner()).insert(path, probe.clone());
    Some(probe)
}

/// The object's bytes on disk and what they are.
pub async fn object(data_dir: &Path, id: &str) -> Option<(PathBuf, Probe)> {
    let id = parse_id(id)?;
    let probe = probe(data_dir, id).await?;
    let path = object_path(data_dir, id, &probe.ext);
    tokio::fs::metadata(&path).await.ok()?.is_file().then_some((path, probe))
}

/// Copy the file at `path` in as an attachment. Resolving a relative path is the caller's
/// — it knows which task's folder the name is relative to.
pub async fn place(data_dir: &Path, path: &Path) -> Result<Placed, Refusal> {
    let shown = path.display().to_string();
    let started = Instant::now();
    let meta = tokio::fs::metadata(path).await.map_err(|_| Refusal::Missing(shown.clone()))?;
    if !meta.is_file() {
        return Err(Refusal::NotAFile(shown));
    }
    if meta.len() > MAX_BYTES {
        return Err(Refusal::TooLarge(shown, meta.len()));
    }
    let source = tokio::fs::canonicalize(path).await.map_err(|_| Refusal::Missing(shown.clone()))?;
    if let Ok(secrets) =
        tokio::fs::canonicalize(data_dir.join("drive").join("accounts").join("secrets")).await
        && source.starts_with(&secrets)
    {
        return Err(Refusal::Secret);
    }

    // Copy first, then read the copy: what is hashed, probed and kept is one set of bytes,
    // even if the worker rewrites its file while this runs.
    let objects = root(data_dir).join("objects");
    let staged = objects.join(format!(".placing-{}", uuid::Uuid::now_v7()));
    let copied = async {
        tokio::fs::create_dir_all(&objects).await?;
        tokio::fs::copy(&source, &staged).await?;
        anyhow::Ok(())
    };
    if let Err(error) = copied.await {
        let _ = tokio::fs::remove_file(&staged).await;
        tracing::warn!(target: "attachments", %error, path = %shown, "could not copy a file in");
        return Err(Refusal::Missing(shown));
    }

    finish(data_dir, &staged, &source, shown, started).await
}

/// Put bytes the agent just made into the store: a generated picture or a clip downloaded
/// from the vendor that made it. There is no file to copy — the bytes *are* what was made —
/// so `named` is only what a refusal calls it, and what its extension is guessed from before
/// the probe decides.
pub async fn place_bytes(data_dir: &Path, bytes: &[u8], named: &str) -> Result<Placed, Refusal> {
    let started = Instant::now();
    if bytes.len() as u64 > MAX_BYTES {
        return Err(Refusal::TooLarge(named.to_owned(), bytes.len() as u64));
    }
    let objects = root(data_dir).join("objects");
    let staged = objects.join(format!(".placing-{}", uuid::Uuid::now_v7()));
    let written = async {
        tokio::fs::create_dir_all(&objects).await?;
        tokio::fs::write(&staged, bytes).await
    };
    if let Err(error) = written.await {
        let _ = tokio::fs::remove_file(&staged).await;
        tracing::warn!(target: "attachments", %error, named, "could not write bytes in");
        return Err(Refusal::Missing(named.to_owned()));
    }
    finish(data_dir, &staged, Path::new(named), named.to_owned(), started).await
}

/// Hash, probe, file and announce what is staged — the half [`place`] and [`place_bytes`]
/// share, from the copy onwards.
async fn finish(
    data_dir: &Path,
    staged: &Path,
    source: &Path,
    shown: String,
    started: Instant,
) -> Result<Placed, Refusal> {
    let result = settle(data_dir, staged, source).await;
    let _ = tokio::fs::remove_file(staged).await;
    let (placed, preview) = match result {
        Ok(done) => done,
        Err(why) => return Err(Refusal::NotShowable(shown, why)),
    };
    tracing::info!(
        target: "attachments",
        id = %placed.id,
        kind = placed.probe.kind.as_str(),
        bytes = placed.probe.bytes,
        preview,
        elapsed_ms = started.elapsed().as_millis() as u64,
        "attached"
    );
    // **Placing is the write that mirrors it** (`docs/arch/topology.md` § *Content*): the
    // bytes cross the uplink while nobody is looking, so by the time the person opens Home on
    // their phone the picture is at the edge. Free when mirroring is off.
    crate::foundation::mirror::enqueue(&url(&placed.id));
    if preview {
        crate::foundation::mirror::enqueue(&preview_url(&placed.id));
    }
    // A clip no browser can play is kept as it is and a copy is made beside it, off the call:
    // a transcode takes seconds to minutes, and the worker's line is not held for it.
    if !placed.probe.playable() {
        ensure_proxy(data_dir.to_owned(), placed.id.clone());
    }
    Ok(placed)
}

/// Hash, probe and file what was staged. The error is the half-sentence a refusal ends
/// with, because a file that does not decode is something the caller did, not a fault here.
async fn settle(
    data_dir: &Path,
    staged: &Path,
    source: &Path,
) -> Result<(Placed, bool), String> {
    let sha = sha256_file(staged).await.map_err(|_| "could not be read".to_owned())?;
    let id = sha[..ID_LEN].to_owned();
    let named_ext = source
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .unwrap_or_default();
    let bytes = tokio::fs::metadata(staged).await.map(|m| m.len()).unwrap_or(0);

    let (probe, frame) = match read_picture(staged.to_owned()).await {
        Ok(Some((format, frame))) => {
            let (ext, _) = picture_type(format).ok_or_else(|| NOT_A_BROWSER_PICTURE.to_owned())?;
            let probe = Probe {
                kind: Kind::Picture,
                ext: ext.to_owned(),
                bytes,
                width: frame.width(),
                height: frame.height(),
                duration_ms: None,
                fps: None,
                codec: None,
                sha256: sha.clone(),
            };
            (probe, Some(frame))
        }
        // Not a picture the image decoder knows; the clip decoder is the other thing that
        // can say what these bytes are — once the first bytes say they are a container a clip
        // comes in. A worker's notes or a PDF never gets as far as spawning ffmpeg.
        Ok(None) | Err(_) => {
            if !looks_like_a_clip(staged).await {
                return Err(NOT_SHOWABLE.to_owned());
            }
            let (clip, frame) = read_clip(staged).await?;
            let ext = clip_ext(&clip.container, &named_ext).ok_or_else(|| NOT_A_BROWSER_CLIP.to_owned())?;
            let probe = Probe {
                kind: Kind::Clip,
                ext: ext.to_owned(),
                bytes,
                width: clip.width,
                height: clip.height,
                duration_ms: Some(clip.duration_ms),
                fps: clip.fps,
                codec: clip.codec,
                sha256: sha.clone(),
            };
            (probe, frame)
        }
    };

    let object = object_path(data_dir, &id, &probe.ext);
    if tokio::fs::metadata(&object).await.is_err() {
        let filed = async {
            tokio::fs::create_dir_all(object.parent().context("object has a parent")?).await?;
            tokio::fs::rename(staged, &object).await?;
            anyhow::Ok(())
        };
        filed.await.map_err(|_| "could not be filed".to_owned())?;
    }
    write_probe(data_dir, &id, &probe).await.map_err(|_| "could not be filed".to_owned())?;

    // The preview is made now rather than queued: it is the tile, and a tile is what a
    // line carrying a picture is for. A failure is logged and leaves the tile to draw the
    // kind in words; the route makes one on first read.
    let preview = match frame {
        Some(frame) => write_preview(data_dir, &id, frame).await.is_ok(),
        None => false,
    };
    Ok((Placed { id, probe }, preview))
}

const NOT_A_BROWSER_PICTURE: &str =
    "is a picture in a format a browser cannot show — attach a PNG, JPEG, WebP or GIF";
const NOT_A_BROWSER_CLIP: &str =
    "is a clip in a container this cannot keep — attach an MP4, MOV, WebM or MKV";
const NOT_SHOWABLE: &str = "is not a picture or a clip — only pictures (PNG, JPEG, WebP, GIF) \
     and clips (MP4, MOV, WebM, MKV) can be attached for now; a recording or a document cannot yet";

async fn sha256_file(path: &Path) -> anyhow::Result<String> {
    let path = path.to_owned();
    tokio::task::spawn_blocking(move || {
        use std::io::Read as _;
        let mut file = std::fs::File::open(&path)?;
        let mut hasher = Sha256::new();
        let mut buf = vec![0u8; 1 << 20];
        loop {
            let n = file.read(&mut buf)?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
        }
        let digest = hasher.finalize();
        anyhow::Ok(digest.iter().map(|b| format!("{b:02x}")).collect::<String>())
    })
    .await?
}

async fn write_probe(data_dir: &Path, id: &str, probe: &Probe) -> anyhow::Result<()> {
    let path = probe_path(data_dir, id);
    if tokio::fs::metadata(&path).await.is_err() {
        write_atomic(&path, &serde_json::to_vec_pretty(probe)?).await?;
    }
    PROBES.lock().unwrap_or_else(|p| p.into_inner()).insert(path, probe.clone());
    Ok(())
}

async fn write_atomic(path: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    let dir = path.parent().context("path has a parent")?;
    tokio::fs::create_dir_all(dir).await?;
    let tmp = dir.join(format!(".writing-{}", uuid::Uuid::now_v7()));
    tokio::fs::write(&tmp, bytes).await?;
    tokio::fs::rename(&tmp, path).await?;
    Ok(())
}

/// The picture formats a browser this product ships in can draw, with the extension and
/// type each is kept as. `None` for one the decoder reads and a browser does not.
fn picture_type(format: ImageFormat) -> Option<(&'static str, &'static str)> {
    match format {
        ImageFormat::Png => Some(("png", "image/png")),
        ImageFormat::Jpeg => Some(("jpg", "image/jpeg")),
        ImageFormat::WebP => Some(("webp", "image/webp")),
        ImageFormat::Gif => Some(("gif", "image/gif")),
        _ => None,
    }
}

/// Decode `path` as a picture — oriented the way a camera's EXIF says it is held — or
/// `Ok(None)` when the image decoder does not recognise the bytes at all.
async fn read_picture(path: PathBuf) -> anyhow::Result<Option<(ImageFormat, DynamicImage)>> {
    tokio::task::spawn_blocking(move || {
        let reader = image::ImageReader::open(&path)?.with_guessed_format()?;
        let Some(format) = reader.format() else {
            return Ok(None);
        };
        let mut decoder = reader.into_decoder()?;
        let orientation = decoder.orientation().unwrap_or(image::metadata::Orientation::NoTransforms);
        let mut frame = DynamicImage::from_decoder(decoder)?;
        frame.apply_orientation(orientation);
        Ok(Some((format, frame)))
    })
    .await?
}

/// Whether the first bytes are one of the containers a clip this accepts comes in: the ISO
/// family (`ftyp` at offset 4 — MP4, MOV, M4V; older QuickTime opens on `moov`, `mdat`, `wide`
/// or `free`) or EBML (WebM, Matroska). A sniff, not a verdict: what the bytes are is still
/// decided by decoding them, and this only keeps what cannot be a clip from being handed to a
/// decoder at all.
async fn looks_like_a_clip(path: &Path) -> bool {
    use tokio::io::AsyncReadExt as _;
    let mut head = [0u8; 12];
    let Ok(mut file) = tokio::fs::File::open(path).await else {
        return false;
    };
    let Ok(n) = file.read(&mut head).await else {
        return false;
    };
    let head = &head[..n];
    head.starts_with(&[0x1a, 0x45, 0xdf, 0xa3])
        || head.get(4..8).is_some_and(|box_type| {
            [b"ftyp", b"moov", b"mdat", b"wide", b"free"].iter().any(|t| box_type == *t)
        })
}

/// What ffmpeg said about a clip on its way to decoding one frame of it.
#[derive(Debug, Clone, PartialEq)]
struct ClipInfo {
    container: String,
    duration_ms: u64,
    width: u32,
    height: u32,
    fps: Option<f64>,
    codec: Option<String>,
}

/// Decode `path` as a clip: its facts, and one representative frame for the preview.
///
/// One ffmpeg run does both. There is no ffprobe in what this product provisions, and the
/// run that draws the preview has to open the clip anyway — **so the check that these bytes
/// really are a clip is that ffmpeg decoded a frame of them**, not what the name ends in.
/// `thumbnail` picks the most representative of the opening frames, which is what keeps a
/// clip that fades in from black from being a black tile.
async fn read_clip(path: &Path) -> Result<(ClipInfo, Option<DynamicImage>), String> {
    let out = Command::new(crate::foundation::vendors::ffmpeg_frame::ffmpeg_bin())
        .args(["-hide_banner", "-nostdin", "-i"])
        .arg(path)
        .args([
            "-map",
            "0:v:0",
            "-vf",
            &format!(
                "thumbnail,scale='min({PREVIEW_W},iw)':'min({PREVIEW_H},ih)':force_original_aspect_ratio=decrease:flags=lanczos"
            ),
            "-frames:v",
            "1",
            "-f",
            "image2pipe",
            "-c:v",
            "png",
            "pipe:1",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .map_err(|error| {
            // Not the file's fault, and the refusal must not say it is: on this very machine the
            // first `ffmpeg` on PATH was once an x86_64 build that could not run at all.
            tracing::warn!(target: "attachments", %error, "ffmpeg could not be run");
            format!("could not be read as a clip — ffmpeg does not run on this machine ({error}), so no clip can be attached until it does")
        })?;
    let said = String::from_utf8_lossy(&out.stderr);
    let info = parse_clip_info(&said).ok_or_else(|| NOT_SHOWABLE.to_owned())?;
    // ffmpeg reads single pictures through its image demuxers and gives them a frame's
    // duration; one the image decoder could not read is a picture, not a short clip.
    let still = info.container.ends_with("_pipe") || info.container == "image2";
    if info.duration_ms == 0 || still {
        // A single still ffmpeg can read and the image decoder could not — HEIC, TIFF.
        return Err(NOT_A_BROWSER_PICTURE.to_owned());
    }
    let frame = if out.status.success() && !out.stdout.is_empty() {
        image::load_from_memory(&out.stdout).ok()
    } else {
        None
    };
    if frame.is_none() {
        // No frame decoded: a recording with no picture in it, or a clip ffmpeg could not
        // read past its header. Either way nothing here can show it.
        return Err(NOT_SHOWABLE.to_owned());
    }
    Ok((info, frame))
}

/// Read the input half of ffmpeg's banner: the container, the duration, and the first video
/// stream's codec, size and rate. `None` when there is no video stream in it.
///
/// Only the lines before the output is described are read — ffmpeg reports the PNG it is
/// writing as a video stream too, and that one is not the clip's.
fn parse_clip_info(stderr: &str) -> Option<ClipInfo> {
    let input = stderr.split("Output #0").next().unwrap_or(stderr);
    let input = input.split("Stream mapping:").next().unwrap_or(input);
    let mut container = None;
    let mut duration_ms = 0u64;
    let mut video: Option<(Option<String>, u32, u32, Option<f64>)> = None;
    let mut quarter_turned = false;
    for line in input.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("Input #0, ") {
            container = rest.split(", from").next().map(str::to_owned);
        } else if let Some(rest) = line.strip_prefix("Duration: ") {
            duration_ms = parse_duration(rest.split(',').next().unwrap_or("")).unwrap_or(0);
        } else if video.is_none()
            && line.starts_with("Stream #0:")
            && let Some(rest) = line.split_once("Video: ").map(|(_, r)| r)
        {
            let codec = rest
                .split([' ', ','])
                .next()
                .filter(|c| !c.is_empty())
                .map(str::to_owned);
            let mut size = None;
            let mut fps = None;
            for part in rest.split(", ") {
                let head = part.split(' ').next().unwrap_or("");
                if size.is_none()
                    && let Some((w, h)) = head.split_once('x')
                    && let (Ok(w), Ok(h)) = (w.parse::<u32>(), h.parse::<u32>())
                {
                    size = Some((w, h));
                }
                if let Some(rate) = part.strip_suffix(" fps") {
                    fps = rate.trim().parse::<f64>().ok();
                }
            }
            let (w, h) = size?;
            video = Some((codec, w, h, fps));
        } else if line.contains("rotation of 90") || line.contains("rotation of -90") {
            // A phone clip held upright is stored sideways with a display matrix; the frame
            // ffmpeg decodes is already turned, so the size is too.
            quarter_turned = true;
        }
    }
    let (codec, mut width, mut height, fps) = video?;
    if quarter_turned {
        std::mem::swap(&mut width, &mut height);
    }
    Some(ClipInfo {
        container: container.unwrap_or_default(),
        duration_ms,
        width,
        height,
        fps,
        codec,
    })
}

/// The video codecs a browser this product ships in can play. A clip in anything else is kept
/// as it is and played from a copy the host makes (`proxy.v1`, `docs/arch/showing.md`
/// § *Derivations*): the court-calibration overlay was MPEG-4 Part 2 (`mp4v`), which neither
/// Chrome nor WebKit decodes, and its worker had to re-encode it to H.264 before the page it
/// built could play it.
const BROWSER_CODECS: &[&str] = &["h264", "hevc", "vp8", "vp9", "av1"];

/// `00:00:30.00` → 30000.
fn parse_duration(s: &str) -> Option<u64> {
    let mut parts = s.trim().split(':');
    let h: f64 = parts.next()?.parse().ok()?;
    let m: f64 = parts.next()?.parse().ok()?;
    let sec: f64 = parts.next()?.parse().ok()?;
    Some(((h * 3600.0 + m * 60.0 + sec) * 1000.0).round() as u64)
}

/// The extension a clip is kept as, from what ffmpeg says its container is and what the
/// file was called — `None` for a container a browser this product ships in cannot play.
/// ffmpeg names the whole QuickTime family one way (`mov,mp4,m4a,…`), so which of the two
/// it is comes from the name.
fn clip_ext(container: &str, named: &str) -> Option<&'static str> {
    let family: Vec<&str> = container.split(',').map(str::trim).collect();
    if family.contains(&"mov") || family.contains(&"mp4") {
        return Some(if named == "mov" { "mov" } else { "mp4" });
    }
    if family.contains(&"webm") || family.contains(&"matroska") {
        return Some(if named == "mkv" { "mkv" } else { "webm" });
    }
    None
}

/// The playable copy of a clip no browser can play as it is: H.264 in MP4, SDR, at most
/// 1080p, `faststart` so it begins before it has all arrived. Versioned like every derivation.
pub const PROXY_SPEC: &str = "proxy.v1";

/// Where a clip is played from, whatever it is kept as — the one URL every surface hands a
/// `<video>`. The route answers with the original when a browser can play it, with the copy once
/// it exists, and with *try again shortly* while it is being made.
pub fn playable_url(id: &str) -> String {
    format!("/api/attachments/{id}/playable")
}

/// Where the copy is served. Immutable, like the object: its bytes are fixed by the id and
/// the spec.
pub fn proxy_url(id: &str) -> String {
    format!("/api/attachments/{id}/{PROXY_SPEC}")
}

fn proxy_path(data_dir: &Path, id: &str) -> PathBuf {
    root(data_dir).join("derived").join(PROXY_SPEC).join(shard(id)).join(format!("{id}.mp4"))
}

/// What playing `id` means right now.
pub enum Playable {
    /// A browser plays it as it is.
    Original,
    /// It is played from its copy, which is on disk.
    Copy(PathBuf),
    /// The copy is being made.
    Preparing,
}

/// Where `id` plays from, asking for its copy if it has none and none is being made — which is
/// how a copy lost to a restart mid-transcode is made again: on the first attempt to play it.
pub async fn playable(data_dir: &Path, id: &str) -> Option<Playable> {
    let id = parse_id(id)?;
    let probe = probe(data_dir, id).await?;
    if probe.playable() {
        return Some(Playable::Original);
    }
    let copy = proxy_path(data_dir, id);
    if tokio::fs::metadata(&copy).await.is_ok() {
        return Some(Playable::Copy(copy));
    }
    ensure_proxy(data_dir.to_owned(), id.to_owned());
    Some(Playable::Preparing)
}

/// Copies being made, by where they will land — so a clip asked for twice is transcoded once.
static MAKING: LazyLock<Mutex<std::collections::HashSet<PathBuf>>> = LazyLock::new(Default::default);

/// At most two transcodes at once, so a long one never starves the machine running the agent,
/// and never holds up a preview, which is made on the call and never queued behind this.
static TRANSCODES: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(2);

/// Make `id`'s copy in the background unless it exists or is already being made.
pub fn ensure_proxy(data_dir: PathBuf, id: String) {
    let copy = proxy_path(&data_dir, &id);
    if copy.exists() || !MAKING.lock().unwrap_or_else(|p| p.into_inner()).insert(copy.clone()) {
        return;
    }
    tokio::spawn(async move {
        let started = Instant::now();
        let made = match TRANSCODES.acquire().await {
            Ok(_slot) => make_proxy(&data_dir, &id, &copy).await,
            Err(_) => Err(anyhow::anyhow!("the transcode queue is closed")),
        };
        MAKING.lock().unwrap_or_else(|p| p.into_inner()).remove(&copy);
        match made {
            Ok(()) => {
                tracing::info!(
                    target: "attachments", %id, elapsed_ms = started.elapsed().as_millis() as u64,
                    "made a copy browsers can play"
                );
                crate::foundation::mirror::enqueue(&proxy_url(&id));
            }
            Err(error) => tracing::warn!(target: "attachments", %id, %error, "could not make a playable copy"),
        }
    });
}

async fn make_proxy(data_dir: &Path, id: &str, copy: &Path) -> anyhow::Result<()> {
    let (object, _) = object(data_dir, id).await.context("no such attachment")?;
    let dir = copy.parent().context("copy has a parent")?;
    tokio::fs::create_dir_all(dir).await?;
    let tmp = dir.join(format!(".making-{}.mp4", uuid::Uuid::now_v7()));
    let out = Command::new(crate::foundation::vendors::ffmpeg_frame::ffmpeg_bin())
        .args(["-hide_banner", "-nostdin", "-loglevel", "error", "-y", "-i"])
        .arg(&object)
        .args([
            "-map", "0:v:0", "-map", "0:a:0?",
            "-vf", "scale='min(1920,iw)':-2",
            "-c:v", "libx264", "-preset", "veryfast", "-crf", "23", "-pix_fmt", "yuv420p",
            "-c:a", "aac", "-b:a", "128k",
            "-movflags", "+faststart",
        ])
        .arg(&tmp)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .await
        .context("ffmpeg could not be run")?;
    if !out.status.success() {
        let _ = tokio::fs::remove_file(&tmp).await;
        bail!("ffmpeg: {}", String::from_utf8_lossy(&out.stderr).trim());
    }
    tokio::fs::rename(&tmp, copy).await?;
    Ok(())
}

/// The preview for `id` on disk and its type, making it from the object if it is missing.
pub async fn preview(data_dir: &Path, id: &str) -> Option<(PathBuf, &'static str)> {
    let id = parse_id(id)?;
    if let Some(found) = existing_preview(data_dir, id).await {
        return Some(found);
    }
    let (object, probe) = object(data_dir, id).await?;
    let frame = match probe.kind {
        Kind::Picture => read_picture(object).await.ok().flatten().map(|(_, f)| f),
        Kind::Clip => read_clip(&object).await.ok().and_then(|(_, f)| f),
    }?;
    write_preview(data_dir, id, frame).await.ok()?;
    existing_preview(data_dir, id).await
}

async fn existing_preview(data_dir: &Path, id: &str) -> Option<(PathBuf, &'static str)> {
    let dir = preview_dir(data_dir, id);
    for (ext, mime) in [("jpg", "image/jpeg"), ("png", "image/png")] {
        let path = dir.join(format!("{id}.{ext}"));
        if tokio::fs::metadata(&path).await.is_ok() {
            return Some((path, mime));
        }
    }
    None
}

/// Fit `frame` inside the preview box — never enlarging it — and store it as JPEG, or as
/// PNG where it has transparency a JPEG would paint black.
async fn write_preview(data_dir: &Path, id: &str, frame: DynamicImage) -> anyhow::Result<()> {
    let dir = preview_dir(data_dir, id);
    let id = id.to_owned();
    let (bytes, ext) = tokio::task::spawn_blocking(move || encode_preview(frame)).await??;
    write_atomic(&dir.join(format!("{id}.{ext}")), &bytes).await
}

fn encode_preview(frame: DynamicImage) -> anyhow::Result<(Vec<u8>, &'static str)> {
    let fitted = if frame.width() > PREVIEW_W || frame.height() > PREVIEW_H {
        frame.resize(PREVIEW_W, PREVIEW_H, image::imageops::FilterType::Lanczos3)
    } else {
        frame
    };
    let mut out = std::io::Cursor::new(Vec::new());
    if fitted.color().has_alpha() {
        fitted.write_to(&mut out, ImageFormat::Png)?;
        return Ok((out.into_inner(), "png"));
    }
    let rgb = fitted.to_rgb8();
    let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, PREVIEW_QUALITY);
    encoder.encode_image(&rgb)?;
    if out.get_ref().is_empty() {
        bail!("the preview encoded to nothing");
    }
    Ok((out.into_inner(), "jpg"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn picture(dir: &Path, name: &str, w: u32, h: u32, seed: u8) -> PathBuf {
        let img = image::RgbImage::from_fn(w, h, |x, y| {
            image::Rgb([(x % 251) as u8 ^ seed, (y % 241) as u8, seed])
        });
        let path = dir.join(name);
        img.save(&path).unwrap();
        path
    }

    #[tokio::test]
    async fn a_picture_is_copied_named_by_its_bytes_and_given_a_tile_sized_preview() {
        let data = tempfile::tempdir().unwrap();
        let work = tempfile::tempdir().unwrap();
        let path = picture(work.path(), "pose_899.png", 1920, 1080, 7);

        let placed = place(data.path(), &path).await.unwrap();
        assert_eq!(placed.id.len(), ID_LEN);
        assert_eq!(placed.probe.kind, Kind::Picture);
        assert_eq!((placed.probe.width, placed.probe.height), (1920, 1080));
        assert_eq!(placed.probe.mime(), "image/png");
        assert_eq!(&placed.probe.sha256[..ID_LEN], placed.id);

        let (object, _) = object(data.path(), &placed.id).await.unwrap();
        assert_eq!(std::fs::read(&object).unwrap(), std::fs::read(&path).unwrap());

        let (preview_file, mime) = preview(data.path(), &placed.id).await.unwrap();
        assert_eq!(mime, "image/jpeg");
        let (w, h) = image::image_dimensions(&preview_file).unwrap();
        assert_eq!((w, h), (960, 540), "fitted inside the tile, aspect kept");
    }

    #[tokio::test]
    async fn the_line_keeps_what_was_true_when_the_worker_overwrites_its_file() {
        let data = tempfile::tempdir().unwrap();
        let work = tempfile::tempdir().unwrap();
        let path = picture(work.path(), "figure.png", 64, 36, 1);
        let first = place(data.path(), &path).await.unwrap();
        let kept = std::fs::read(object(data.path(), &first.id).await.unwrap().0).unwrap();

        picture(work.path(), "figure.png", 64, 36, 2);
        let second = place(data.path(), &path).await.unwrap();
        assert_ne!(first.id, second.id, "new bytes are a new attachment");
        assert_eq!(
            std::fs::read(object(data.path(), &first.id).await.unwrap().0).unwrap(),
            kept,
            "and the first one still shows what it showed"
        );
    }

    #[tokio::test]
    async fn the_same_bytes_twice_are_one_attachment() {
        let data = tempfile::tempdir().unwrap();
        let work = tempfile::tempdir().unwrap();
        let a = picture(work.path(), "a.png", 40, 30, 3);
        let b = work.path().join("copy-of-a.png");
        std::fs::copy(&a, &b).unwrap();
        assert_eq!(
            place(data.path(), &a).await.unwrap().id,
            place(data.path(), &b).await.unwrap().id
        );
        let objects: Vec<_> = walk(&data.path().join("attachments/objects"));
        assert_eq!(objects.iter().filter(|p| p.ends_with(".png")).count(), 1, "{objects:?}");
        assert!(objects.iter().all(|p| !p.contains(".placing-")), "nothing staged is left behind: {objects:?}");
    }

    #[tokio::test]
    async fn a_small_picture_is_not_enlarged_and_transparency_stays_png() {
        let data = tempfile::tempdir().unwrap();
        let work = tempfile::tempdir().unwrap();
        let path = work.path().join("icon.png");
        image::RgbaImage::from_pixel(120, 80, image::Rgba([10, 20, 30, 128])).save(&path).unwrap();
        let placed = place(data.path(), &path).await.unwrap();
        let (file, mime) = preview(data.path(), &placed.id).await.unwrap();
        assert_eq!(mime, "image/png");
        assert_eq!(image::image_dimensions(file).unwrap(), (120, 80));
    }

    #[tokio::test]
    async fn what_is_not_a_picture_or_a_clip_is_refused_and_nothing_is_kept() {
        let data = tempfile::tempdir().unwrap();
        let work = tempfile::tempdir().unwrap();
        let notes = work.path().join("NOTES.md");
        std::fs::write(&notes, "# notes\nthe court is the near one").unwrap();
        let refused = place(data.path(), &notes).await.unwrap_err();
        assert!(matches!(refused, Refusal::NotShowable(_, _)), "{refused:?}");
        assert!(refused.to_string().contains("for now"), "it says what can be attached: {refused}");
        assert!(walk(&data.path().join("attachments/objects")).is_empty());

        assert!(matches!(
            place(data.path(), &work.path().join("gone.png")).await.unwrap_err(),
            Refusal::Missing(_)
        ));
        assert!(matches!(place(data.path(), work.path()).await.unwrap_err(), Refusal::NotAFile(_)));
    }

    #[tokio::test]
    async fn a_stored_secret_is_never_attached() {
        let data = tempfile::tempdir().unwrap();
        let secrets = data.path().join("drive/accounts/secrets");
        std::fs::create_dir_all(&secrets).unwrap();
        let path = picture(&secrets, "totp-qr.png", 32, 32, 9);
        assert_eq!(place(data.path(), &path).await.unwrap_err(), Refusal::Secret);
    }

    #[tokio::test]
    async fn an_id_nobody_attached_is_nothing() {
        let data = tempfile::tempdir().unwrap();
        assert!(probe(data.path(), "att:0123456789abcdef").await.is_none());
        assert!(probe(data.path(), "../../etc/passwd").await.is_none());
        assert_eq!(parse_id("att:0123456789abcdef"), Some("0123456789abcdef"));
        assert_eq!(parse_id("0123456789ABCDEF"), None, "an id is lowercase hex, exactly");
        assert_eq!(parse_id("att:0123"), None);
    }

    #[test]
    fn a_clip_is_read_off_the_input_half_of_the_banner() {
        // The court-calibration source, as the bundled ffmpeg describes it, followed by the
        // PNG this run writes — whose own "Video:" line must not be read as the clip's.
        let said = "\
Input #0, mov,mp4,m4a,3gp,3g2,mj2, from '/Users/x/Desktop/bad/raw/381_raw_000000-000030.mp4':
  Metadata:
    major_brand     : isom
  Duration: 00:00:30.00, start: 0.000000, bitrate: 5994 kb/s
  Stream #0:0[0x1](und): Video: h264 (High) (avc1 / 0x31637661), yuv420p(tv, bt2020nc/bt2020/arib-std-b67, progressive), 1920x1080 [SAR 1:1 DAR 16:9], 5775 kb/s, 30 fps, 30 tbr, 30k tbn (default)
  Stream #0:1[0x2](und): Audio: aac (LC) (mp4a / 0x6134706D), 48000 Hz, stereo, fltp, 191 kb/s (default)
Stream mapping:
  Stream #0:0 -> #0:0 (h264 (native) -> png (native))
Output #0, image2pipe, to 'pipe:1':
  Stream #0:0: Video: png, rgb24, 960x540 [SAR 1:1 DAR 16:9], q=2-31, 200 kb/s, 30 fps, 30 tbn
";
        let info = parse_clip_info(said).unwrap();
        assert_eq!(info.duration_ms, 30_000);
        assert_eq!((info.width, info.height), (1920, 1080));
        assert_eq!(info.fps, Some(30.0));
        assert_eq!(info.codec.as_deref(), Some("h264"));
        assert_eq!(clip_ext(&info.container, "mp4"), Some("mp4"));
        assert_eq!(clip_ext(&info.container, "mov"), Some("mov"));
        assert_eq!(clip_ext("avi", "avi"), None);
    }

    #[test]
    fn a_clip_a_browser_cannot_play_is_kept_and_played_from_a_copy() {
        // The court-calibration overlay, as ffmpeg describes it.
        let said = "\
Input #0, mov,mp4,m4a,3gp,3g2,mj2, from 'marked_silent.mp4':
  Duration: 00:00:30.00, start: 0.000000, bitrate: 10462 kb/s
  Stream #0:0[0x1](und): Video: mpeg4 (Simple Profile) (mp4v / 0x7634706D), yuv420p, 1920x1080 [SAR 1:1 DAR 16:9], 10461 kb/s, 30 fps, 30 tbr, 15360 tbn (default)
";
        let info = parse_clip_info(said).unwrap();
        let probe = |codec: &str, ext: &str| Probe {
            kind: Kind::Clip,
            ext: ext.into(),
            bytes: 1,
            width: info.width,
            height: info.height,
            duration_ms: Some(info.duration_ms),
            fps: info.fps,
            codec: Some(codec.into()),
            sha256: String::new(),
        };
        let overlay = probe(info.codec.as_deref().unwrap(), "mp4");
        assert!(!overlay.playable(), "mpeg4 is kept, and played from a copy");
        assert!(overlay.describe().ends_with("a copy browsers can play is being made"), "{}", overlay.describe());
        assert!(probe("h264", "mp4").playable());
        assert!(!probe("h264", "mkv").playable(), "the container matters too");
    }

    #[test]
    fn an_upright_phone_clip_is_as_tall_as_it_is_held() {
        let said = "\
Input #0, mov,mp4,m4a,3gp,3g2,mj2, from 'IMG_0001.MOV':
  Duration: 00:00:07.43, start: 0.000000, bitrate: 11000 kb/s
  Stream #0:0[0x1](und): Video: hevc (Main) (hvc1 / 0x31637668), yuv420p(tv, bt709), 1920x1080, 10000 kb/s, 29.97 fps, 29.97 tbr, 600 tbn (default)
      Side data:
        displaymatrix: rotation of -90.00 degrees
";
        let info = parse_clip_info(said).unwrap();
        assert_eq!((info.width, info.height), (1080, 1920));
        assert_eq!(info.duration_ms, 7_430);
        assert_eq!(info.fps, Some(29.97));
    }

    #[test]
    fn a_recording_has_no_video_stream_to_read() {
        let said = "\
Input #0, mp3, from 'memo.mp3':
  Duration: 00:01:02.00, start: 0.025057, bitrate: 128 kb/s
  Stream #0:0: Audio: mp3 (mp3float), 44100 Hz, stereo, fltp, 128 kb/s
";
        assert_eq!(parse_clip_info(said), None);
    }

    #[tokio::test]
    async fn only_what_opens_like_a_clip_is_handed_to_the_decoder() {
        let dir = tempfile::tempdir().unwrap();
        let file = |name: &str, bytes: &[u8]| {
            let path = dir.path().join(name);
            std::fs::write(&path, bytes).unwrap();
            path
        };
        let mp4 = file("a.mp4", b"\x00\x00\x00\x20ftypisom\x00\x00\x02\x00");
        let webm = file("a.webm", b"\x1a\x45\xdf\xa3\x9f\x42\x86\x81\x01");
        let notes = file("NOTES.md", b"# notes\nthe court is the near one");
        let pdf = file("report.pdf", b"%PDF-1.7\n");
        assert!(looks_like_a_clip(&mp4).await);
        assert!(looks_like_a_clip(&webm).await);
        assert!(!looks_like_a_clip(&notes).await);
        assert!(!looks_like_a_clip(&pdf).await);
    }

    /// **An attachment goes on the stage through a module the host writes, not one a builder
    /// made** — three lines naming the object, importing the face's own viewer by the bare
    /// specifiers the import map resolves.
    #[tokio::test]
    async fn an_attachment_goes_on_the_stage_as_itself() {
        let data = tempfile::tempdir().unwrap();
        let work = tempfile::tempdir().unwrap();
        let placed = place(data.path(), &picture(work.path(), "pose_899.png", 320, 180, 5)).await.unwrap();
        let module = stage_module(&placed.id, &placed.probe);
        assert!(module.contains("from \"@hi/core\"") && module.contains("AttachmentStage"), "{module}");
        assert!(module.contains(&format!("\"url\":\"/api/attachments/{}\"", placed.id)), "{module}");
        assert_eq!(stage_module_url(&placed.id), format!("/api/attachments/{}/stage.v1.mjs", placed.id));
        // What a view that embeds it by id is told is the item the stage module was written from.
        let about = about(&placed.id, &placed.probe);
        assert!(module.contains(&about.to_string()), "{module}\n{about}");
        assert_eq!(about["kind"], "picture");
        assert_eq!((about["width"].as_u64(), about["height"].as_u64()), (Some(320), Some(180)));
        assert_eq!(about_url(&placed.id), format!("/api/attachments/{}/about.v1", placed.id));

        assert_eq!(ref_id(&format!("att:{}", placed.id)), Some(placed.id.as_str()));
        assert_eq!(ref_id(&placed.id), None, "a bare id is not a ref: a view's ref could look like one");
        assert_eq!(ref_id("factory/tasks"), None);
        assert_eq!(label(data.path(), &placed.id), "Picture");
        assert_eq!(label(data.path(), "0123456789abcdef"), "Attachment");
    }

    #[test]
    fn a_probe_describes_itself_in_one_line() {
        let probe = Probe {
            kind: Kind::Clip,
            ext: "mp4".into(),
            bytes: 1,
            width: 1920,
            height: 1080,
            duration_ms: Some(30_000),
            fps: Some(29.97),
            codec: Some("h264".into()),
            sha256: String::new(),
        };
        assert_eq!(probe.describe(), "clip 1920×1080 · 0:30 · 29.97 fps");
        assert_eq!(clock(3_725_000), "1:02:05");
    }

    /// A real clip through the real ffmpeg. Ignored in `make test` because it needs the
    /// binary on this machine; `make test-live` runs it.
    #[tokio::test]
    #[ignore]
    async fn a_real_clip_is_attached_with_its_facts_and_a_preview() {
        let data = tempfile::tempdir().unwrap();
        let work = tempfile::tempdir().unwrap();
        let clip = work.path().join("court.mp4");
        let made = std::process::Command::new(crate::foundation::vendors::ffmpeg_frame::ffmpeg_bin())
            .args(["-hide_banner", "-loglevel", "error", "-f", "lavfi", "-i", "testsrc=duration=2:size=1280x720:rate=30", "-pix_fmt", "yuv420p"])
            .arg(&clip)
            .status()
            .unwrap();
        assert!(made.success());
        let placed = place(data.path(), &clip).await.unwrap();
        eprintln!("{:?}", placed.probe);
        assert_eq!(placed.probe.kind, Kind::Clip);
        assert_eq!((placed.probe.width, placed.probe.height), (1280, 720));
        assert_eq!(placed.probe.duration_ms, Some(2_000));
        assert_eq!(placed.probe.mime(), "video/mp4");
        let (file, _) = preview(data.path(), &placed.id).await.unwrap();
        assert_eq!(image::image_dimensions(file).unwrap(), (960, 540));
    }

    /// A clip no browser decodes, through the real ffmpeg: kept as it is, and played from the
    /// copy made beside it. Ignored in `make test` because it needs the binary and a transcode.
    #[tokio::test]
    #[ignore]
    async fn an_unplayable_clip_is_played_from_the_copy_made_beside_it() {
        let data = tempfile::tempdir().unwrap();
        let work = tempfile::tempdir().unwrap();
        let clip = work.path().join("marked_silent.mp4");
        let made = std::process::Command::new(crate::foundation::vendors::ffmpeg_frame::ffmpeg_bin())
            .args(["-hide_banner", "-loglevel", "error", "-f", "lavfi", "-i", "testsrc=duration=2:size=640x360:rate=30", "-c:v", "mpeg4"])
            .arg(&clip)
            .status()
            .unwrap();
        assert!(made.success());
        let placed = place(data.path(), &clip).await.unwrap();
        assert_eq!(placed.probe.codec.as_deref(), Some("mpeg4"));
        assert!(!placed.probe.playable());
        let mut waited = 0;
        loop {
            match playable(data.path(), &placed.id).await.unwrap() {
                Playable::Copy(path) => {
                    assert!(std::fs::metadata(path).unwrap().len() > 0);
                    break;
                }
                Playable::Preparing if waited < 120 => {
                    tokio::time::sleep(std::time::Duration::from_millis(250)).await;
                    waited += 1;
                }
                _ => panic!("no copy after 30 s"),
            }
        }
    }

    fn walk(dir: &Path) -> Vec<String> {
        let mut out = Vec::new();
        let Ok(entries) = std::fs::read_dir(dir) else {
            return out;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                out.extend(walk(&path));
            } else {
                out.push(path.display().to_string());
            }
        }
        out
    }
}
