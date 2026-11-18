use std::io;
use std::path::PathBuf;
use std::time::Duration;

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
}
