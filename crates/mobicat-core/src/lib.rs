//! Platform-independent core: the normalized log entry, the entry store, and
//! output formatting.

pub mod annotate;
pub mod clock;
pub mod entry;
pub mod group;
pub mod intern;
pub mod output;
pub mod query;
pub mod store;

pub use annotate::{Annotator, Noise};
pub use clock::Clock;
pub use entry::{Entry, Framework, Level, Platform, Source};
pub use group::Grouper;
pub use intern::Interner;
pub use query::{Query, QueryError};
pub use store::{Device, Filter, SELF_DEVICE, Seq, Store, View};
