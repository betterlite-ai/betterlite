// the three POST endpoints LiteLLM registers under both /v1beta/models and /models
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Operation {
    GenerateContent,
    StreamGenerateContent,
    CountTokens,
}

// FastAPI's {model_name:path} is greedy up to the route's literal suffix, so the last colon decides
#[must_use]
pub fn split(rest: &str) -> Option<(&str, Operation)> {
    let (model, suffix) = rest.rsplit_once(':')?;
    let operation = match suffix {
        "generateContent" => Operation::GenerateContent,
        "streamGenerateContent" => Operation::StreamGenerateContent,
        "countTokens" => Operation::CountTokens,
        _other => return None,
    };
    Some((model, operation))
}
