use std::io;
use std::path::PathBuf;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum SourceError {
    #[error("the batch file {} could not be opened", .path.display())]
    Open {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("the batch upload handle must be seekable so it can be read from the start")]
    NotSeekable,
    #[error("the batch upload handle could not seek to the start")]
    Seek(#[source] io::Error),
    #[error("the batch file could not be read")]
    Read(#[source] io::Error),
    #[error("a batch line is longer than {limit} bytes")]
    LineTooLong { limit: usize },
}
