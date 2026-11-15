#![cfg(test)]

use std::convert::Infallible;
use std::fs;
use std::io::{self, Cursor, SeekFrom};
use std::path::Path;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::task::{Context, Poll};

use betterlite_prompts::arguments::json::Json;
use futures_util::StreamExt as _;
use rstest::rstest;
use tokio::io::{AsyncRead, AsyncSeek, BufReader, ReadBuf};

use crate::vertex_batch_upload::sources::error::SourceError;
use crate::vertex_batch_upload::sources::{FileContent, FileTypes, LineReader};
use crate::vertex_batch_upload::tests::{MAX_LINE_BYTES, Settings, context, upload};
use crate::vertex_batch_upload::upload::error::UploadError;
use crate::vertex_batch_upload::upload::{probe_first_entry, vertex_batch_upload};

#[derive(Default)]
struct Counters {
    seeks: AtomicUsize,
    reads: AtomicUsize,
    closed: AtomicBool,
}

struct Tracked {
    inner: Cursor<Vec<u8>>,
    counters: Arc<Counters>,
}

impl AsyncRead for Tracked {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        self.counters.reads.fetch_add(1, Ordering::SeqCst);
        Pin::new(&mut self.inner).poll_read(cx, buf)
    }
}

impl AsyncSeek for Tracked {
    fn start_seek(mut self: Pin<&mut Self>, position: SeekFrom) -> io::Result<()> {
        self.counters.seeks.fetch_add(1, Ordering::SeqCst);
        Pin::new(&mut self.inner).start_seek(position)
    }

    fn poll_complete(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<u64>> {
        Pin::new(&mut self.inner).poll_complete(cx)
    }
}

impl Drop for Tracked {
    fn drop(&mut self) {
        self.counters.closed.store(true, Ordering::SeqCst);
    }
}

struct Refusing;

impl AsyncRead for Refusing {
    fn poll_read(
        self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        _buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}

impl AsyncSeek for Refusing {
    fn start_seek(self: Pin<&mut Self>, _position: SeekFrom) -> io::Result<()> {
        Err(io::Error::other("seek refused"))
    }

    fn poll_complete(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<u64>> {
        Poll::Ready(Ok(0))
    }
}

fn row(custom_id: &str, padding: usize) -> String {
    let text = "x".repeat(padding);
    format!(r#"{{"custom_id":"{custom_id}","url":"/v1/embeddings","body":{{"input":"{text}"}}}}"#)
}

fn tracked(content: &str) -> (FileTypes, Arc<Counters>) {
    let counters = Arc::new(Counters::default());
    let handle = Tracked {
        inner: Cursor::new(content.as_bytes().to_vec()),
        counters: Arc::clone(&counters),
    };
    (
        FileTypes::Content(FileContent::Handle(Box::new(handle))),
        counters,
    )
}

fn joined(chunks: Vec<Result<Vec<u8>, UploadError<Infallible>>>) -> String {
    String::from_utf8(chunks.into_iter().flat_map(Result::unwrap).collect()).unwrap()
}

fn is_open(path: &Path) -> bool {
    let target = fs::canonicalize(path).unwrap();
    fs::read_dir("/proc/self/fd")
        .unwrap()
        .filter_map(|entry| fs::read_link(entry.ok()?.path()).ok())
        .any(|link| link == target)
}

#[tokio::test]
async fn first_row_rewind() {
    let content = format!("{}\n{}", row("first", 1), row("second", 1));
    let (mut file, counters) = tracked(&content);
    let probed = probe_first_entry(&mut file, MAX_LINE_BYTES).await.unwrap();
    assert_eq!(probed.unwrap().custom_id.as_deref(), Some("first"));
    let text = joined(upload(file, "batch", &Json::Bool(false), usize::MAX).await);
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 2);
    assert_eq!(text.matches('\n').count(), 1);
    assert!(!text.ends_with('\n'));
    assert!(lines.first().unwrap().starts_with(r#"{"key": "first""#));
    assert!(lines.last().unwrap().starts_with(r#"{"key": "second""#));
    assert_eq!(counters.seeks.load(Ordering::SeqCst), 2);
}

#[rstest]
#[case::refused_seek(
    FileContent::Handle(Box::new(Refusing)),
    "the batch upload handle could not seek to the start"
)]
#[case::no_seek(
    FileContent::Unseekable(Box::new(Cursor::new(Vec::<u8>::new()))),
    "the batch upload handle must be seekable so it can be read from the start"
)]
#[tokio::test]
async fn failed_seek(#[case] content: FileContent, #[case] message: &str) {
    let mut file = FileTypes::Content(content);
    let probe = probe_first_entry(&mut file, MAX_LINE_BYTES).await;
    assert_eq!(probe.unwrap_err().to_string(), message);
    let chunks = upload(file, "batch", &Json::Bool(false), 1).await;
    assert_eq!(
        chunks.first().unwrap().as_ref().unwrap_err().to_string(),
        message
    );
}

#[tokio::test]
async fn lazy_next_row() {
    let content = format!("{}\nnot json\n{}", row("a", 5000), row("b", 5000));
    let (file, counters) = tracked(&content);
    let one = upload(file, "batch", &Json::Bool(false), 1).await;
    assert_eq!(one.len(), 1);
    assert!(one.first().unwrap().is_ok());
    assert_eq!(counters.reads.load(Ordering::SeqCst), 1);
    assert!(counters.closed.load(Ordering::SeqCst));
    let (second, _counters) = tracked(&content);
    let two = upload(second, "batch", &Json::Bool(false), 2).await;
    assert!(matches!(two.last(), Some(Err(UploadError::Line(_)))));
}

#[tokio::test]
async fn local_path_close() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("batch.jsonl");
    fs::write(&path, format!("{}\n{}", row("a", 1), row("b", 1))).unwrap();
    let settings = Settings::default();
    let context = context(&settings);
    let file = FileTypes::Content(FileContent::Path(path.clone()));
    let mut stream = Box::pin(vertex_batch_upload(
        file,
        "batch",
        &Json::Bool(false),
        &context,
        MAX_LINE_BYTES,
    ));
    stream.next().await.unwrap().unwrap();
    assert!(is_open(&path));
    drop(stream);
    assert!(!is_open(&path));
}

#[tokio::test]
async fn missing_path() {
    let missing = FileContent::Path(Path::new("/nonexistent/batch.jsonl").to_path_buf());
    let chunks = upload(FileTypes::Content(missing), "batch", &Json::Bool(false), 1).await;
    let error = chunks.first().unwrap().as_ref().unwrap_err();
    assert!(matches!(
        error,
        UploadError::Source(SourceError::Open { .. })
    ));
}

#[tokio::test]
async fn text_is_never_a_path() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("batch.jsonl");
    fs::write(&path, row("a", 1)).unwrap();
    let text = path.to_str().unwrap().to_owned();
    let chunks = upload(
        FileTypes::Content(FileContent::Text(text)),
        "batch",
        &Json::Bool(false),
        1,
    )
    .await;
    assert!(matches!(chunks.first(), Some(Err(UploadError::Line(_)))));
}

#[tokio::test]
async fn tuple_payload_uses_its_content() {
    let file = FileTypes::Tuple {
        filename: "batch.jsonl".to_owned(),
        content: FileContent::Text(row("a", 1)),
    };
    let text = joined(upload(file, "batch", &Json::Bool(false), usize::MAX).await);
    assert!(text.starts_with(r#"{"key": "a""#));
}

#[rstest]
#[case::two_full_and_one(FileContent::Bytes(vec![b'x'; 0x0020_0001]), &[0x0010_0000, 0x0010_0000, 1])]
#[case::exactly_one(FileContent::Bytes(vec![b'x'; 0x0010_0000]), &[0x0010_0000])]
#[case::one_past(FileContent::Bytes(vec![b'x'; 0x0010_0001]), &[0x0010_0000, 1])]
#[case::empty(FileContent::Bytes(Vec::new()), &[])]
#[case::text_is_one_chunk(FileContent::Text("x".repeat(0x0018_0000)), &[0x0018_0000])]
#[case::handle(FileContent::Handle(Box::new(Cursor::new(vec![b'x'; 0x0010_0005]))), &[0x0010_0000, 5])]
#[tokio::test]
async fn raw_chunk_boundaries(#[case] content: FileContent, #[case] sizes: &[usize]) {
    let chunks = upload(
        FileTypes::Content(content),
        "batch",
        &Json::Bool(true),
        usize::MAX,
    )
    .await;
    let lengths: Vec<usize> = chunks
        .into_iter()
        .map(|chunk| chunk.unwrap().len())
        .collect();
    assert_eq!(lengths, sizes);
}

#[rstest]
#[case::at_the_limit(6, "abcdef", Some("abcdef"))]
#[case::past_the_limit(5, "abcdef", None)]
#[case::newline_does_not_count(6, "abcdef\nxy", Some("abcdef"))]
#[tokio::test]
async fn line_limit(#[case] limit: usize, #[case] input: &str, #[case] expected: Option<&str>) {
    let mut lines = LineReader {
        reader: BufReader::new(Cursor::new(input.as_bytes().to_vec())),
        limit,
    };
    let read = lines.next_line().await;
    match expected {
        Some(text) => assert_eq!(read.unwrap().unwrap(), text.as_bytes()),
        None => assert!(matches!(read, Err(SourceError::LineTooLong { limit: 5 }))),
    }
}
