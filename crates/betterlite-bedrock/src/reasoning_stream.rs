pub mod buffered;
pub mod refusals;
pub mod stream;

#[cfg(test)]
mod tests;

pub const REASONING_OPEN_TAG: &str = "<reasoning>";
pub const REASONING_CLOSE_TAG: &str = "</reasoning>";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ReasoningPhase {
    #[default]
    Start,
    Reasoning,
    AfterClose,
    Content,
}

impl ReasoningPhase {
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Start => "start",
            Self::Reasoning => "reasoning",
            Self::AfterClose => "after_close",
            Self::Content => "content",
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ReasoningTagState {
    pub phase: ReasoningPhase,
    pub pending: String,
}

impl ReasoningTagState {
    #[must_use]
    pub fn feed(&self, text: &str) -> (Self, String, String) {
        match self.phase {
            ReasoningPhase::Content => (self.clone(), String::new(), text.to_owned()),
            ReasoningPhase::AfterClose => {
                // str.lstrip also strips U+001C..U+001F, four characters Unicode does not treat as whitespace
                let content = text
                    .trim_start_matches(|character: char| {
                        character.is_whitespace() || matches!(character, '\u{1c}'..='\u{1f}')
                    })
                    .to_owned();
                if content.is_empty() {
                    (self.clone(), String::new(), content)
                } else {
                    let next = Self {
                        phase: ReasoningPhase::Content,
                        pending: String::new(),
                    };
                    (next, String::new(), content)
                }
            }
            ReasoningPhase::Start => {
                let mut buffered = self.pending.clone();
                buffered.push_str(text);
                feed_start(buffered)
            }
            ReasoningPhase::Reasoning => {
                let mut buffered = self.pending.clone();
                buffered.push_str(text);
                feed_reasoning(buffered)
            }
        }
    }

    #[must_use]
    pub fn flush(&self) -> (Self, String, String) {
        let drained = Self {
            phase: ReasoningPhase::Content,
            pending: String::new(),
        };
        if self.phase == ReasoningPhase::Reasoning {
            (drained, self.pending.clone(), String::new())
        } else {
            (drained, String::new(), self.pending.clone())
        }
    }
}

#[must_use]
pub fn decode_reasoning_delta(
    state: &ReasoningTagState,
    content: Option<&str>,
    finished: bool,
) -> (ReasoningTagState, String, String) {
    let (fed_state, fed_reasoning, fed_content) = state.feed(content.unwrap_or_default());
    if !finished {
        return (fed_state, fed_reasoning, fed_content);
    }
    let (drained, flushed_reasoning, flushed_content) = fed_state.flush();
    let mut reasoning = fed_reasoning;
    reasoning.push_str(&flushed_reasoning);
    let mut body = fed_content;
    body.push_str(&flushed_content);
    (drained, reasoning, body)
}

#[must_use]
pub fn split_reasoning_tag(content: &str) -> (Option<String>, String) {
    let (_, reasoning, body) =
        decode_reasoning_delta(&ReasoningTagState::default(), Some(content), true);
    ((!reasoning.is_empty()).then_some(reasoning), body)
}

fn feed_start(buffered: String) -> (ReasoningTagState, String, String) {
    if let Some(rest) = buffered.strip_prefix(REASONING_OPEN_TAG) {
        let entered = ReasoningTagState {
            phase: ReasoningPhase::Reasoning,
            pending: String::new(),
        };
        return entered.feed(rest);
    }
    if REASONING_OPEN_TAG.starts_with(buffered.as_str()) {
        let pending = ReasoningTagState {
            phase: ReasoningPhase::Start,
            pending: buffered,
        };
        return (pending, String::new(), String::new());
    }
    let content = ReasoningTagState {
        phase: ReasoningPhase::Content,
        pending: String::new(),
    };
    (content, String::new(), buffered)
}

fn feed_reasoning(buffered: String) -> (ReasoningTagState, String, String) {
    if let Some(at) = buffered.find(REASONING_CLOSE_TAG) {
        let (reasoning, closed) = buffered.split_at(at);
        let rest = closed.strip_prefix(REASONING_CLOSE_TAG).unwrap_or(closed);
        let after_close = ReasoningTagState {
            phase: ReasoningPhase::AfterClose,
            pending: String::new(),
        };
        let (next, _, content) = after_close.feed(rest);
        return (next, reasoning.to_owned(), content);
    }
    let (kept, held) = buffered.split_at(held_close_tag_start(&buffered));
    let state = ReasoningTagState {
        phase: ReasoningPhase::Reasoning,
        pending: held.to_owned(),
    };
    (state, kept.to_owned(), String::new())
}

// The byte offset where the longest trailing proper prefix of the close tag starts, or the text length when there is none.
fn held_close_tag_start(text: &str) -> usize {
    let limit = REASONING_CLOSE_TAG.chars().count().saturating_sub(1);
    let mut starts: Vec<usize> = text
        .char_indices()
        .map(|(offset, _character)| offset)
        .rev()
        .take(limit)
        .collect();
    starts.reverse();
    for start in starts {
        let (_, suffix) = text.split_at(start);
        if REASONING_CLOSE_TAG.starts_with(suffix) {
            return start;
        }
    }
    text.len()
}
