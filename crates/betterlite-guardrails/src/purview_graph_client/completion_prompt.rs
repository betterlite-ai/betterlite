use betterlite_prompts::arguments::json::Json;

fn is_token_id(value: &Json) -> bool {
    match value {
        Json::Number(number) => number.is_i64() || number.is_u64(),
        Json::Null | Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_) => false,
    }
}

// True when an OpenAI /v1/completions prompt carries token ids, which hold no plaintext for Purview.
#[must_use]
pub fn is_token_id_prompt(prompt: Option<&Json>) -> bool {
    let Some(Json::List(items)) = prompt else {
        return false;
    };
    items.iter().any(|item| match item {
        Json::List(inner) => inner.iter().any(is_token_id),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_) => {
            is_token_id(item)
        }
    })
}

#[must_use]
pub fn completion_prompt_to_str(prompt: Option<&Json>) -> Option<String> {
    let text = match prompt? {
        Json::Text(text) => text.trim().to_owned(),
        Json::List(items) => {
            if !items.is_empty() && items.iter().all(is_token_id) {
                tracing::debug!(
                    "Purview DLP: completions prompt is token ids only; skipping text scan"
                );
                return None;
            }
            let strings: Vec<&str> = items
                .iter()
                .filter_map(Json::as_str)
                .map(str::trim)
                .collect();
            strings.join("\n").trim().to_owned()
        }
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Object(_) => return None,
    };
    (!text.is_empty()).then_some(text)
}

mod tests;
