use std::collections::BTreeMap;
use std::str::FromStr as _;

use betterlite_models::catalog::{CatalogEntry, ModelCatalog};
use betterlite_models::decisions::Json;
use betterlite_models::metadata::identity::AudioFormat;
use betterlite_models::metadata::vendor::VertexAudioApi;
use rust_decimal::Decimal;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LyriaModelInfo {
    pub vertex_ai_audio_api: VertexAudioApi,
    pub supported_audio_formats: Vec<AudioFormat>,
    pub output_cost_per_image: Option<Decimal>,
}

#[must_use]
pub fn lyria_model_key(model: &str) -> String {
    if model.starts_with("vertex_ai/") {
        model.to_owned()
    } else {
        format!("vertex_ai/{model}")
    }
}

// LiteLLM validates runtime model-cost entries with a pydantic adapter and drops an entry that fails
#[must_use]
pub fn validate_lyria_model_info(raw: &Json) -> Option<LyriaModelInfo> {
    let Json::Object(fields) = raw else {
        return None;
    };
    let audio_api = match fields.get("vertex_ai_audio_api")? {
        Json::Text(text) if text == "lyria_predict" => VertexAudioApi::LyriaPredict,
        Json::Text(text) if text == "lyria_interactions" => VertexAudioApi::LyriaInteractions,
        Json::Null
        | Json::Bool(_)
        | Json::Number(_)
        | Json::Text(_)
        | Json::List(_)
        | Json::Object(_) => return None,
    };
    let Json::List(formats) = fields.get("supported_audio_formats")? else {
        return None;
    };
    let supported_audio_formats = formats
        .iter()
        .map(audio_format)
        .collect::<Option<Vec<_>>>()?;
    let output_cost_per_image = match fields.get("output_cost_per_image") {
        None => None,
        Some(value) => Some(cost_value(value)?),
    };
    Some(LyriaModelInfo {
        vertex_ai_audio_api: audio_api,
        supported_audio_formats,
        output_cost_per_image,
    })
}

fn audio_format(value: &Json) -> Option<AudioFormat> {
    let Json::Text(text) = value else {
        return None;
    };
    match text.as_str() {
        "mp3" => Some(AudioFormat::Mp3),
        "wav" => Some(AudioFormat::Wav),
        _ => None,
    }
}

fn cost_value(value: &Json) -> Option<Decimal> {
    match value {
        Json::Number(number) => {
            let text = number.to_string();
            Decimal::from_str(&text)
                .or_else(|_error| Decimal::from_scientific(&text))
                .ok()
        }
        Json::Text(text) => Decimal::from_str(text)
            .or_else(|_error| Decimal::from_scientific(text))
            .ok(),
        Json::Bool(true) => Some(Decimal::ONE),
        Json::Bool(false) => Some(Decimal::ZERO),
        Json::Null | Json::List(_) | Json::Object(_) => None,
    }
}

fn bundled_info(entry: &CatalogEntry) -> Option<LyriaModelInfo> {
    let audio_api = entry.fields.metadata.vertex_ai_audio_api?;
    Some(LyriaModelInfo {
        vertex_ai_audio_api: audio_api,
        supported_audio_formats: entry.fields.metadata.supported_audio_formats.clone(),
        output_cost_per_image: entry.prices.values.output.output_image,
    })
}

pub struct LyriaOverlay<'sources> {
    pub runtime: &'sources BTreeMap<String, Json>,
    pub bundled: &'sources ModelCatalog,
}

impl LyriaOverlay<'_> {
    #[must_use]
    pub fn model_info(&self, model: &str) -> Option<LyriaModelInfo> {
        let key = lyria_model_key(model);
        if let Some(runtime) = self.runtime.get(&key).and_then(validate_lyria_model_info) {
            return Some(runtime);
        }
        let entry = self.bundled.entries.get(&key)?;
        bundled_info(entry)
    }

    // the runtime entry's price can be absent while its info is valid, so this falls back on its own
    #[must_use]
    pub fn generation_cost(&self, model: &str) -> Option<Decimal> {
        let key = lyria_model_key(model);
        if let Some(runtime) = self.runtime.get(&key).and_then(validate_lyria_model_info)
            && let Some(cost) = runtime.output_cost_per_image
        {
            return Some(cost);
        }
        let info = bundled_info(self.bundled.entries.get(&key)?)?;
        info.output_cost_per_image
    }
}
