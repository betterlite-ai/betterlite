use std::collections::VecDeque;
use std::sync::Arc;

use betterlite_openai::aleph_alpha::lines::LineDecoder;
use betterlite_openai::stream::chunk::Chunk;
use futures_util::StreamExt as _;
use tokio::time::timeout;

use super::error::LangGraphStreamError;
use super::iterator::{ChunkShape, LineOutcome, make_chunk, parse_sse_line};
use super::{Chunks, ERROR_LOG, IdSource, STREAMING_TIMEOUT};

pub struct LineReader {
    pub chunks: Chunks,
    pub decoder: LineDecoder,
    pub lines: VecDeque<String>,
    // an encoding_rs decoder flushes once and panics if used again, so the end of the body stays set
    pub done: bool,
}

impl LineReader {
    pub async fn next_line(&mut self) -> Result<Option<String>, LangGraphStreamError> {
        loop {
            if let Some(line) = self.lines.pop_front() {
                return Ok(Some(line));
            }
            if self.done {
                return Ok(None);
            }
            match timeout(STREAMING_TIMEOUT, self.chunks.next()).await {
                Ok(Some(Ok(bytes))) => self.decoder.push(&bytes, &mut self.lines)?,
                Ok(Some(Err(source))) => return Err(LangGraphStreamError::Body { source }),
                Ok(None) => {
                    self.decoder.flush(&mut self.lines);
                    self.done = true;
                }
                Err(_elapsed) => return Err(LangGraphStreamError::Timeout),
            }
        }
    }
}

pub struct LangGraphSseStream {
    pub model: String,
    pub ids: Arc<dyn IdSource>,
    pub lines: Option<LineReader>,
    pub finished: bool,
}

enum Advancement {
    Chunk(Chunk),
    // a metadata run_id chunk, or the chunk a stream without one gets at its end
    Finished(Chunk),
    Skipped,
    End,
}

impl LangGraphSseStream {
    pub async fn next_chunk(&mut self) -> Option<Chunk> {
        loop {
            match self.advance().await {
                Advancement::Skipped => {}
                Advancement::Chunk(chunk) | Advancement::Finished(chunk) => return Some(chunk),
                Advancement::End => return None,
            }
        }
    }

    async fn advance(&mut self) -> Advancement {
        let line = {
            let Some(lines) = self.lines.as_mut() else {
                return Advancement::End;
            };
            lines.next_line().await
        };
        match line {
            Ok(Some(text)) => self.arrive(&text),
            Ok(None) => {
                self.lines = None;
                self.finish()
            }
            Err(error) => self.fail(&error),
        }
    }

    fn arrive(&mut self, line: &str) -> Advancement {
        match parse_sse_line(line, &self.model, self.ids.as_ref()) {
            Ok(LineOutcome::Chunk(chunk)) => Advancement::Chunk(chunk),
            Ok(LineOutcome::Final(chunk)) => {
                self.finished = true;
                Advancement::Finished(chunk)
            }
            Ok(LineOutcome::Skipped) => Advancement::Skipped,
            Err(error) => self.fail(&error),
        }
    }

    fn finish(&mut self) -> Advancement {
        if self.finished {
            return Advancement::End;
        }
        self.finished = true;
        Advancement::Finished(make_chunk(
            &self.model,
            self.ids.as_ref(),
            ChunkShape::Final,
        ))
    }

    fn fail(&mut self, error: &LangGraphStreamError) -> Advancement {
        tracing::error!("{ERROR_LOG}: {error}");
        self.lines = None;
        Advancement::End
    }
}
