//! One append-only jsonl file, written without blocking and read back from its tail.
//!
//! Two things in this directory need exactly this and nothing more: the session index
//! ([`super::index`]) and the mail log ([`super::mail`]). Both are appended from synchronous
//! paths that must not touch the filesystem — `register` and `unregister` run inside the
//! switchboard lock, and `unregister` is reachable from a `Drop` where there is no `await` to
//! be had — and both are read once at boot from a bounded tail, because an install that has
//! been up for months must not pay a whole file to answer a question about this afternoon.
//!
//! **Evidence, never a dependency.** A write that fails is logged and dropped; a disk that has
//! stopped accepting them is not something a retry here can fix, and neither file may take the
//! agent down with it.

use std::path::{Path, PathBuf};

use serde::Serialize;
use tokio::sync::mpsc;

/// The writer half: an unbounded channel drained by one task.
pub struct Writer {
    tx: mpsc::UnboundedSender<Msg>,
    /// Only for the log line when a send fails, so an operator knows *which* file went quiet.
    what: &'static str,
}

/// What crosses to the append task. A flush travels as a message rather than on its own
/// channel so that it takes its place in the queue: the loop is FIFO, so a reply to a `Flush`
/// proves every line queued before it is already on disk.
enum Msg {
    Line(String),
    Flush(tokio::sync::oneshot::Sender<()>),
}

impl Writer {
    /// Start the writer for `path`, spawning its append task.
    pub fn start(path: PathBuf, what: &'static str) -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        tokio::spawn(append_loop(path, rx));
        Self { tx, what }
    }

    /// Queue one record, serialized here rather than in the append task.
    ///
    /// Serializing on the caller's thread is what keeps the failure *nameable*: in the loop
    /// all that is left is a value nobody can describe, while here the caller still has the
    /// record and the type. It is a small struct either way.
    pub fn write<R: Serialize>(&self, record: &R) {
        let line = match serde_json::to_string(record) {
            Ok(line) => line,
            Err(err) => {
                tracing::error!(error = %err, what = self.what, "a record would not serialize");
                return;
            }
        };
        if self.tx.send(Msg::Line(line)).is_err() {
            tracing::warn!(what = self.what, "the writer is gone; a record went unrecorded");
        }
    }

    /// Wait until everything queued so far has reached the disk.
    ///
    /// **The shutdown path cannot do without this.** Closing the switchboard queues a record
    /// per live session and the process then exits at once; with no flush the runtime drops
    /// the append task while those are still in the channel, and a clean stop would leave
    /// exactly the `opened`-with-no-`closed` pattern that means *crashed*.
    pub async fn flush(&self) {
        let (tx, rx) = tokio::sync::oneshot::channel();
        if self.tx.send(Msg::Flush(tx)).is_err() {
            return;
        }
        // A dropped sender means the loop is gone, which is as flushed as this will get.
        let _ = rx.await;
    }
}

/// Append lines as they arrive, batching whatever is already queued into one write.
async fn append_loop(path: PathBuf, mut rx: mpsc::UnboundedReceiver<Msg>) {
    let mut batch: Vec<String> = Vec::new();
    // Flush replies are held until after the write below, never answered on receipt — a reply
    // that outran the `write_all` would be a promise this cannot keep.
    let mut waiting: Vec<tokio::sync::oneshot::Sender<()>> = Vec::new();

    while let Some(first) = rx.recv().await {
        match first {
            Msg::Line(l) => batch.push(l),
            Msg::Flush(tx) => waiting.push(tx),
        }
        while let Ok(more) = rx.try_recv() {
            match more {
                Msg::Line(l) => batch.push(l),
                Msg::Flush(tx) => waiting.push(tx),
            }
        }

        let mut buf = String::new();
        for line in batch.drain(..) {
            buf.push_str(&line);
            buf.push('\n');
        }
        if !buf.is_empty() {
            append(&path, &buf).await;
        }

        // **Always**, on every path out of the write — including a failed one and an empty
        // batch. A `flush` that is never answered parks the caller forever, and its one caller
        // is the shutdown path, so an unanswered reply is a process that will not exit.
        for tx in waiting.drain(..) {
            let _ = tx.send(());
        }
    }
}

async fn append(path: &Path, buf: &str) {
    use tokio::io::AsyncWriteExt as _;

    if let Some(parent) = path.parent()
        && let Err(err) = tokio::fs::create_dir_all(parent).await
    {
        tracing::error!(error = %err, path = %parent.display(), "cannot make the directory");
        return;
    }
    match tokio::fs::OpenOptions::new().create(true).append(true).open(path).await {
        Ok(mut f) => {
            if let Err(err) = f.write_all(buf.as_bytes()).await {
                tracing::error!(error = %err, path = %path.display(), "append failed");
            }
        }
        Err(err) => tracing::error!(error = %err, path = %path.display(), "cannot open for append"),
    }
}

/// Read at most `limit` bytes from the end of `path`, starting at a line boundary.
///
/// Seeking into the middle of a line is expected — the first partial line is dropped by
/// starting after the first newline. A file shorter than `limit` is read whole and keeps its
/// first line. A missing file reads as empty: a fresh install has neither of these, and that
/// is not a failure.
pub async fn read_tail(path: &Path, limit: u64) -> String {
    use tokio::io::{AsyncReadExt as _, AsyncSeekExt as _};

    let read = async {
        let mut f = tokio::fs::File::open(path).await?;
        let len = f.metadata().await?.len();
        let whole = len <= limit;
        if !whole {
            f.seek(std::io::SeekFrom::Start(len - limit)).await?;
        }
        let mut buf = Vec::with_capacity(limit.min(len) as usize);
        f.read_to_end(&mut buf).await?;
        let text = String::from_utf8_lossy(&buf).into_owned();
        Ok::<_, std::io::Error>(match whole {
            true => text,
            // Drop the partial first line.
            false => text.find('\n').map(|i| text[i + 1..].to_string()).unwrap_or_default(),
        })
    };
    match read.await {
        Ok(text) => text,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(err) => {
            tracing::warn!(error = %err, path = %path.display(), "cannot read the tail");
            String::new()
        }
    }
}
