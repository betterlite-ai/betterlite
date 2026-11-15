use std::io::Cursor;
use std::path::PathBuf;

use tokio::fs::File;
use tokio::io::{
    AsyncBufReadExt as _, AsyncRead, AsyncReadExt as _, AsyncSeek, AsyncSeekExt as _, BufReader,
};

use crate::vertex_batch_upload::sources::error::SourceError;

pub mod error;

mod tests;

// LiteLLM reads raw uploads in 1 MiB chunks
const RAW_CHUNK_BYTES: u64 = 1_048_576;

pub type Reader = Box<dyn AsyncRead + Send + Unpin>;

pub trait UploadHandle: AsyncRead + AsyncSeek + Send + Unpin {}

impl<Handle> UploadHandle for Handle where Handle: AsyncRead + AsyncSeek + Send + Unpin {}

// A string is literal JSONL text and never a path; only the caller that authorized a path builds Path
pub enum FileContent {
    Bytes(Vec<u8>),
    Text(String),
    Path(PathBuf),
    Handle(Box<dyn UploadHandle>),
    Unseekable(Reader),
}

// B1128's FileTypes: the content alone, or the (filename, content) tuple whose second item is the content
pub enum FileTypes {
    Content(FileContent),
    Tuple {
        filename: String,
        content: FileContent,
    },
}

impl FileTypes {
    pub const fn content_mut(&mut self) -> &mut FileContent {
        match self {
            Self::Content(content) | Self::Tuple { content, .. } => content,
        }
    }

    #[must_use]
    pub fn into_content(self) -> FileContent {
        match self {
            Self::Content(content) | Self::Tuple { content, .. } => content,
        }
    }
}

// a handle is rewound first, so a failed or missing seek is refused before any row is read
pub async fn open_reader(content: FileContent) -> Result<Reader, SourceError> {
    match content {
        FileContent::Bytes(bytes) => Ok(Box::new(Cursor::new(bytes))),
        FileContent::Text(text) => Ok(Box::new(Cursor::new(text.into_bytes()))),
        FileContent::Path(path) => {
            let opened = File::open(&path).await;
            let file = opened.map_err(|source| SourceError::Open { path, source })?;
            Ok(Box::new(file))
        }
        FileContent::Handle(mut handle) => {
            handle.rewind().await.map_err(SourceError::Seek)?;
            Ok(Box::new(handle))
        }
        FileContent::Unseekable(_reader) => Err(SourceError::NotSeekable),
    }
}

pub struct LineReader<Input> {
    pub reader: BufReader<Input>,
    pub limit: usize,
}

impl<Input> LineReader<Input>
where
    Input: AsyncRead + Unpin,
{
    // the line without its LF, read only when asked for; a line past the limit fails instead of growing the buffer
    pub async fn next_line(&mut self) -> Result<Option<Vec<u8>>, SourceError> {
        let mut line = Vec::new();
        let allowed = u64::try_from(self.limit).map_or(u64::MAX, |limit| limit.saturating_add(1));
        let read = (&mut self.reader)
            .take(allowed)
            .read_until(b'\n', &mut line)
            .await
            .map_err(SourceError::Read)?;
        if read == 0 {
            return Ok(None);
        }
        if line.last() == Some(&b'\n') {
            line.pop();
        }
        if line.len() > self.limit {
            return Err(SourceError::LineTooLong { limit: self.limit });
        }
        Ok(Some(line))
    }
}

pub enum RawState {
    Pending(FileContent),
    Reading(Reader),
    Done,
}

// a string is one chunk, everything else comes in full 1 MiB chunks until the source runs dry
pub async fn next_raw_chunk(state: RawState) -> Result<Option<(Vec<u8>, RawState)>, SourceError> {
    let mut reader = match state {
        RawState::Pending(FileContent::Text(text)) => {
            return Ok(Some((text.into_bytes(), RawState::Done)));
        }
        RawState::Pending(content) => open_reader(content).await?,
        RawState::Reading(reader) => reader,
        RawState::Done => return Ok(None),
    };
    let mut chunk = Vec::new();
    (&mut reader)
        .take(RAW_CHUNK_BYTES)
        .read_to_end(&mut chunk)
        .await
        .map_err(SourceError::Read)?;
    if chunk.is_empty() {
        return Ok(None);
    }
    Ok(Some((chunk, RawState::Reading(reader))))
}
