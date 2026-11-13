use std::collections::VecDeque;
use std::sync::{Mutex, PoisonError};

use betterlite_core::api::messages_context_clear::counter::{TokenCountRequest, TokenCounter};
use betterlite_core::api::messages_context_clear::error::ClearToolUsesError;
use betterlite_tokenizer::count::error::CountError;

pub(super) struct ScriptedCounter {
    values: Mutex<VecDeque<usize>>,
}

impl ScriptedCounter {
    pub(super) fn new(values: &[usize]) -> Self {
        let queue = values.iter().copied().collect();
        Self {
            values: Mutex::new(queue),
        }
    }
}

impl TokenCounter for ScriptedCounter {
    fn count(&self, _request: TokenCountRequest<'_>) -> Result<usize, ClearToolUsesError> {
        let mut values = self.values.lock().unwrap_or_else(PoisonError::into_inner);
        values
            .pop_front()
            .ok_or(ClearToolUsesError::TokenCount(CountError::Overflow))
    }
}
