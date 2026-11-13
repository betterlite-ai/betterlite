use std::pin::Pin;
use std::task::{Context, Poll};

use futures_util::Stream;

// LiteLLM's TextStreamer: text.split() keeps bare words only, and one index serves both iteration modes
#[derive(Clone, Debug)]
pub struct TextStreamer {
    words: Vec<String>,
    index: usize,
}

impl TextStreamer {
    #[must_use]
    pub fn new(text: &str) -> Self {
        Self {
            words: text
                .split(|character: char| {
                    character.is_whitespace() || matches!(character, '\u{1c}'..='\u{1f}')
                })
                .filter(|word| !word.is_empty())
                .map(str::to_owned)
                .collect(),
            index: 0,
        }
    }
}

impl Iterator for TextStreamer {
    type Item = String;

    fn next(&mut self) -> Option<Self::Item> {
        let word = self.words.get(self.index)?.clone();
        self.index = self.index.saturating_add(1);
        Some(word)
    }
}

impl Stream for TextStreamer {
    type Item = String;

    fn poll_next(mut self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Poll::Ready(self.next())
    }
}
