use std::io;
use std::path::PathBuf;
use std::time::Duration;

use tokio::task::JoinError;

#[derive(Debug, thiserror::Error)]
pub enum RedisFixtureError {
    #[error("redis-server is not installed; install Debian's redis-server package")]
    MissingServer {
        #[source]
        source: io::Error,
    },
    #[error("the redis-server process could not be started")]
    Spawn {
        #[source]
        source: io::Error,
    },
    #[error("the fixture folder could not be created")]
    Folder {
        #[source]
        source: io::Error,
    },
    #[error("the fixture log could not be opened or read")]
    Log {
        #[source]
        source: io::Error,
    },
    #[error("redis-server did not answer PING within {limit:?}; its log says: {log}")]
    NotReady { limit: Duration, log: String },
    #[error("the fixture socket path {path:?} is not valid UTF-8")]
    NonUtf8Path { path: PathBuf },
    #[error("the blocking redis fixture task ended without a result")]
    BlockingTaskAborted,
    #[error("the blocking redis fixture task panicked or was cancelled")]
    BlockingTaskFailed {
        #[source]
        source: JoinError,
    },
}
