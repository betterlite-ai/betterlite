pub mod collector;
pub mod files;
pub mod merge;
pub mod writer;

use std::io;
use std::path::PathBuf;

// the multiprocess folder, the files in it and the keys they hold, as prometheus_client reports them
#[derive(Debug, thiserror::Error)]
pub enum MultiprocessError {
    #[error("env PROMETHEUS_MULTIPROC_DIR is not set or not a directory")]
    MissingDirectory,
    #[error("cannot list the multiprocess folder {}: {source}", path.display())]
    List {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("cannot read {}: {source}", path.display())]
    Read {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("cannot remove {}: {source}", path.display())]
    Remove {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("Read beyond file size detected, file is corrupted.")]
    Corrupted,
    #[error("the file stops before the entry at offset {offset}")]
    Truncated { offset: usize },
    #[error("the key at offset {offset} is not UTF-8")]
    NonUtf8Key { offset: usize },
    #[error("the key is not a metric key: {source}")]
    InvalidKey {
        #[source]
        source: serde_json::Error,
    },
    #[error("Invalid metric name: {name}")]
    InvalidName { name: String },
    #[error("Invalid metric type: {kind}")]
    InvalidType { kind: String },
    #[error("the multiprocess file name {} does not name a gauge pid", path.display())]
    InvalidFileName { path: PathBuf },
    #[error("the bucket bound {bound} is not a float")]
    InvalidBucketBound { bound: String },
}
