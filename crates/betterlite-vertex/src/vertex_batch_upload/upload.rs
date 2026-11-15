use std::collections::VecDeque;
use std::convert::Infallible;
use std::io::Cursor;

use betterlite_prompts::arguments::json::Json;
use betterlite_prompts::media::inline::RemoteMediaFetcher;
use futures_util::future::Either;
use futures_util::stream::{Stream, TryStreamExt as _, try_unfold};
use tokio::fs::File;
use tokio::io::{AsyncRead, AsyncSeekExt as _, BufReader};

use crate::vertex_batch_upload::line::{BatchLine, decode_openai_batch_line};
use crate::vertex_batch_upload::rows::{RowContext, RowMappers, vertex_rows};
use crate::vertex_batch_upload::sources::error::SourceError;
use crate::vertex_batch_upload::sources::{
    FileContent, FileTypes, LineReader, RawState, Reader, next_raw_chunk, open_reader,
};
use crate::vertex_batch_upload::upload::error::UploadError;

pub mod error;

const PURPOSE_BATCH: &str = "batch";

struct RowState<'ctx, Mappers, Fetcher> {
    context: &'ctx RowContext<'ctx, Mappers, Fetcher>,
    unopened: Option<FileContent>,
    lines: Option<LineReader<Reader>>,
    pending: VecDeque<String>,
    emitted: bool,
    limit: usize,
}

// Raw passthrough needs purpose=batch and a passthrough that is literally true, so 1 or "true" still translates
pub fn vertex_batch_upload<'ctx, Mappers, Fetcher>(
    file: FileTypes,
    purpose: &str,
    passthrough: &Json,
    context: &'ctx RowContext<'ctx, Mappers, Fetcher>,
    max_line_bytes: usize,
) -> impl Stream<Item = Result<Vec<u8>, UploadError<Mappers::Error>>> + use<'ctx, Mappers, Fetcher>
where
    Mappers: RowMappers + Sync,
    Fetcher: RemoteMediaFetcher + Sync,
{
    if purpose == PURPOSE_BATCH && *passthrough == Json::Bool(true) {
        let raw = try_unfold(RawState::Pending(file.into_content()), next_raw_chunk);
        return Either::Left(raw.map_err(UploadError::Source));
    }
    let state = RowState {
        context,
        unopened: Some(file.into_content()),
        lines: None,
        pending: VecDeque::new(),
        emitted: false,
        limit: max_line_bytes,
    };
    Either::Right(try_unfold(state, next_chunk))
}

// one Vertex row per chunk, LF before every row but the first, and the next line is read only once the rows in hand are sent
async fn next_chunk<'ctx, Mappers, Fetcher>(
    mut state: RowState<'ctx, Mappers, Fetcher>,
) -> Result<Option<(Vec<u8>, RowState<'ctx, Mappers, Fetcher>)>, UploadError<Mappers::Error>>
where
    Mappers: RowMappers + Sync,
    Fetcher: RemoteMediaFetcher + Sync,
{
    loop {
        if let Some(row) = state.pending.pop_front() {
            let mut chunk = Vec::with_capacity(row.len().saturating_add(1));
            if state.emitted {
                chunk.push(b'\n');
            }
            chunk.extend_from_slice(row.as_bytes());
            state.emitted = true;
            return Ok(Some((chunk, state)));
        }
        if let Some(content) = state.unopened.take() {
            let reader = open_reader(content).await?;
            state.lines = Some(LineReader {
                reader: BufReader::new(reader),
                limit: state.limit,
            });
        }
        let Some(lines) = state.lines.as_mut() else {
            return Ok(None);
        };
        let Some(raw) = lines.next_line().await? else {
            return Ok(None);
        };
        let Some(entry) = decode_openai_batch_line(&raw)? else {
            continue;
        };
        let rows = vertex_rows(&entry, state.context).await.map_err(Box::new)?;
        state.pending.extend(rows);
    }
}

// The first entry decides the GCS object name, so the probe starts at offset zero and the upload starts there again
pub async fn probe_first_entry(
    file: &mut FileTypes,
    max_line_bytes: usize,
) -> Result<Option<BatchLine>, UploadError<Infallible>> {
    match file.content_mut() {
        FileContent::Bytes(bytes) => {
            first_entry(Cursor::new(bytes.as_slice()), max_line_bytes).await
        }
        FileContent::Text(text) => first_entry(Cursor::new(text.as_bytes()), max_line_bytes).await,
        FileContent::Path(path) => {
            let opened = File::open(path.as_path()).await;
            let reader = opened.map_err(|source| SourceError::Open {
                path: path.clone(),
                source,
            })?;
            first_entry(reader, max_line_bytes).await
        }
        FileContent::Handle(handle) => {
            handle.rewind().await.map_err(SourceError::Seek)?;
            first_entry(handle, max_line_bytes).await
        }
        FileContent::Unseekable(_) => Err(UploadError::Source(SourceError::NotSeekable)),
    }
}

async fn first_entry<Input>(
    input: Input,
    max_line_bytes: usize,
) -> Result<Option<BatchLine>, UploadError<Infallible>>
where
    Input: AsyncRead + Unpin,
{
    let mut lines = LineReader {
        reader: BufReader::new(input),
        limit: max_line_bytes,
    };
    while let Some(raw) = lines.next_line().await? {
        if let Some(entry) = decode_openai_batch_line(&raw)? {
            return Ok(Some(entry));
        }
    }
    Ok(None)
}
