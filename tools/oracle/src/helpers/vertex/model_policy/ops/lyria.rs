use std::collections::BTreeMap;

use betterlite_models::catalog::{BundledCatalog, ModelCatalog};
use betterlite_models::metadata::identity::AudioFormat;
use betterlite_models::metadata::vendor::VertexAudioApi;
use betterlite_vertex::model_policy::lyria::{LyriaModelInfo, LyriaOverlay, lyria_model_key};
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive as _;
use serde_json::{Value, json};

use super::super::input::LyriaCase;

pub(super) fn lyria(cases: &[LyriaCase]) -> Value {
    let catalog = BundledCatalog::default().get();
    let entries: Vec<Value> = cases
        .iter()
        .map(|case| lyria_value(case, catalog.as_ref()))
        .collect();
    json!({"cases": entries})
}

fn lyria_value(case: &LyriaCase, catalog: &ModelCatalog) -> Value {
    let mut runtime = BTreeMap::new();
    if let Some(entry) = &case.runtime {
        runtime.insert(lyria_model_key(&case.model), entry.clone());
    }
    let overlay = LyriaOverlay {
        runtime: &runtime,
        bundled: catalog,
    };
    let info = overlay.model_info(&case.model);
    let cost = overlay.generation_cost(&case.model);
    json!({
        "info": info.as_ref().map_or(Value::Null, info_value),
        "cost": cost.map_or(Value::Null, decimal_value),
    })
}

fn info_value(info: &LyriaModelInfo) -> Value {
    let formats: Vec<&str> = info
        .supported_audio_formats
        .iter()
        .map(|format| audio_format_name(*format))
        .collect();
    json!({
        "audio_api": audio_api_name(info.vertex_ai_audio_api),
        "formats": formats,
        "cost": info.output_cost_per_image.map_or(Value::Null, decimal_value),
    })
}

const fn audio_api_name(api: VertexAudioApi) -> &'static str {
    match api {
        VertexAudioApi::LyriaPredict => "lyria_predict",
        VertexAudioApi::LyriaInteractions => "lyria_interactions",
    }
}

const fn audio_format_name(format: AudioFormat) -> &'static str {
    match format {
        AudioFormat::Mp3 => "mp3",
        AudioFormat::Wav => "wav",
    }
}

fn decimal_value(amount: Decimal) -> Value {
    let Some(float) = amount.to_f64() else {
        return Value::Null;
    };
    json!(float)
}
