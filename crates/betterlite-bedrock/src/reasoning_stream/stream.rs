use std::collections::BTreeMap;

use betterlite_openai::chat_choices::wire::StreamReply;

use super::{ReasoningTagState, decode_reasoning_delta};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StreamingHandler {
    splitters: BTreeMap<i64, ReasoningTagState>,
}

impl StreamingHandler {
    pub fn chunk_parser(&mut self, parsed: &mut StreamReply) {
        for choice in &mut parsed.choices {
            let state = self
                .splitters
                .get(&choice.index)
                .cloned()
                .unwrap_or_default();
            let finished = choice.finish_reason.is_some();
            let (next, reasoning, content) =
                decode_reasoning_delta(&state, choice.delta.content.as_deref(), finished);
            self.splitters.insert(choice.index, next);
            if !reasoning.is_empty() {
                let mut combined = choice.delta.reasoning_content.take().unwrap_or_default();
                combined.push_str(&reasoning);
                choice.delta.reasoning_content = Some(combined);
            }
            if !content.is_empty() || choice.delta.content.is_some() {
                choice.delta.content = Some(content);
            }
        }
    }
}
