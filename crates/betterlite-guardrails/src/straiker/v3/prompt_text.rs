use betterlite_prompts::arguments::json::Json;
use betterlite_tokenizer::engine::assets;
use betterlite_tokenizer::engine::tiktoken::{Encoding, TiktokenEngine};

// The text-davinci-003 tokenizer LiteLLM decodes token ids with before it calls the model. An implementation that has no tokenizer, or an id it cannot decode, answers None.
pub trait PromptTokenDecoder {
    fn decode(&self, ids: &[u32]) -> Option<String>;
}

// text-davinci-003 uses the p50k_base encoding. The vocabulary ships inside betterlite-tokenizer, so a build that fails to load it keeps no engine and decodes nothing, like LiteLLM when tiktoken is missing.
pub struct DavinciDecoder {
    engine: Option<TiktokenEngine>,
}

impl DavinciDecoder {
    #[must_use]
    pub fn new() -> Self {
        let engine = assets::bundled(Encoding::P50kBase)
            .and_then(|data| TiktokenEngine::from_bpe_file(Encoding::P50kBase, &data).ok());
        Self { engine }
    }
}

impl Default for DavinciDecoder {
    fn default() -> Self {
        Self::new()
    }
}

impl PromptTokenDecoder for DavinciDecoder {
    fn decode(&self, ids: &[u32]) -> Option<String> {
        self.engine.as_ref()?.decode(ids).ok()
    }
}

// The text the model receives for a completions `prompt`: a string, a list of strings, a list of token ids, or a list of token-id lists. None for any other shape and for ids that do not decode, so the caller relays the prompt untouched rather than screening something else.
#[must_use]
pub fn prompt_texts(prompt: &Json, decoder: &dyn PromptTokenDecoder) -> Option<Vec<String>> {
    let Json::List(items) = prompt else {
        return prompt.as_str().map(|text| vec![text.to_owned()]);
    };
    if items.is_empty() {
        return None;
    }
    if items.iter().all(|item| matches!(item, Json::Text(_))) {
        return Some(
            items
                .iter()
                .filter_map(Json::as_str)
                .map(str::to_owned)
                .collect(),
        );
    }
    if let Some(ids) = token_ids(prompt) {
        return decoder.decode(&ids).map(|text| vec![text]);
    }
    items
        .iter()
        .map(|item| token_ids(item).and_then(|ids| decoder.decode(&ids)))
        .collect()
}

// A non-empty list of whole numbers that fit a token id. Anything else, a negative or oversized id included, cannot be decoded.
fn token_ids(value: &Json) -> Option<Vec<u32>> {
    let Json::List(items) = value else {
        return None;
    };
    if items.is_empty() {
        return None;
    }
    items
        .iter()
        .map(|item| match item {
            Json::Number(number) => number.as_u64().and_then(|id| u32::try_from(id).ok()),
            Json::Null | Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_) => None,
        })
        .collect()
}

mod tests;
