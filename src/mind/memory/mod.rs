//! Memory substrate — the lossless raw signal store and snapshot building.
//!
//! `Memory` is a cheap-to-clone handle that holds the journal writer. Server
//! handlers and the reaction share one instance. On-disk, signals live under
//! `<data_dir>/memory/raw/` (see [`layout`]); blobs are co-located with the
//! day-log that references them.

use std::path::Path;

pub mod conduct;
pub mod decay;
pub mod episodes;
pub mod facets;
pub mod journal;
pub mod layout;
pub mod media;
pub mod people_vectors;
pub mod proactivity;
pub mod snapshot;
pub mod tasks;

pub use journal::Journal;
pub use snapshot::{Snapshot, build, window};

#[derive(Clone)]
pub struct Memory {
    pub journal: Journal,
}

impl Memory {
    pub async fn open(data_dir: &Path) -> anyhow::Result<Self> {
        let journal = Journal::open(data_dir.to_path_buf()).await?;
        // Fold any legacy flat facets into the one supported shape before anything
        // reads them. Best-effort: a store that cannot be tidied is still a store
        // worth opening, and the next boot tries again.
        if let Err(err) = facets::adopt_flat_facets(data_dir).await {
            tracing::warn!(error = %format!("{err:#}"), "flat-facet adoption failed; leaving them as they are");
        }
        Ok(Self { journal })
    }

    /// The data directory backing this store (root of `<data_dir>/memory/…`).
    pub fn data_dir(&self) -> &Path {
        self.journal.data_dir()
    }
}
