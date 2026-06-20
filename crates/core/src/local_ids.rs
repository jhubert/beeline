//! Local-id mapping now lives in `mailagent-storage` (it is both minted and
//! resolved by the SQLite layer). Re-exported here for a stable path.

pub use mailagent_storage::ProviderRef;
