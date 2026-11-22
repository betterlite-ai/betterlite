#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ResponseUsage {
    pub total_tokens: i64,
}

// the response a finished call logs; only these three kinds count tokens against the minute counters
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CompletionResponse {
    Chat(Option<ResponseUsage>),
    Embedding(Option<ResponseUsage>),
    TextCompletion(Option<ResponseUsage>),
    Other,
}

// _response_total_tokens
#[must_use]
pub fn response_total_tokens(response: &CompletionResponse) -> i64 {
    match response {
        CompletionResponse::Chat(usage)
        | CompletionResponse::Embedding(usage)
        | CompletionResponse::TextCompletion(usage) => {
            usage.map_or(0, |response_usage| response_usage.total_tokens)
        }
        CompletionResponse::Other => 0,
    }
}
