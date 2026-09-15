//! Legibility — the host's half of `docs/arch/legibility.md`.
//!
//! What a person reads is the product, and the writer cannot see its own defaults, so this
//! is where what Reaction writes is read by something other than Reaction:
//!
//! - [`check`] — triage in code, then one model request, between `hi_say` and the floor
//!   (§ D, § E);
//! - [`audit`] — every spoken turn read as it ends, and the person's next message read for
//!   whether it corrects how things were put (§ G).
//!
//! What they find is kept by [`crate::mind::memory::quality`], which Reflection reads to learn
//! grain and conduct (§ H) and which the numbers are computed from (§ I). The standard every
//! one of them judges against is the one Reaction writes against:
//! `src/identity/craft/reading.md`.

pub mod audit;
pub mod check;
pub mod judge;

pub use check::{Brief, Mode, Speech};
