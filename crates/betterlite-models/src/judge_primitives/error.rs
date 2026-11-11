use fancy_regex::Error as RegexError;

// LiteLLM raises the json module's error for a reply with no JSON and a ValueError for a non-object
#[derive(Debug, thiserror::Error)]
pub enum JudgePrimitivesError {
    #[error("the judge reply is not JSON: {source}")]
    Json {
        #[source]
        source: serde_json::Error,
    },
    #[error("judge response is not a JSON object")]
    NotAnObject,
    #[error("the JSON fence pattern did not compile: {source}")]
    FencePattern {
        #[source]
        source: RegexError,
    },
    #[error("scanning the judge reply for a JSON fence failed: {source}")]
    FenceScan {
        #[source]
        source: RegexError,
    },
}
