use std::sync::PoisonError;

use betterlite_openai::image_variations::OpenAiImageVariationConfig;
use betterlite_openai::image_variations::logging::data_view;
use betterlite_openai::image_variations::request::{
    begin_image_variation, finish_image_variation, send_image_variation,
};
use serde_json::{Map, Value, json};

use super::input::Probe;
use super::{FakeTransport, Recorder, call_for, failure_view, image_for};
use crate::helpers::{HelperError, HelperResult};

pub(crate) const NOW: i64 = 1_700_000_000;

pub(crate) fn invalid(message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: String::from("InvalidInput"),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

fn empty_answer() -> Map<String, Value> {
    let mut answer = Map::new();
    for name in [
        "mapped",
        "data",
        "supplied",
        "kind",
        "result",
        "error",
        "pre_call",
        "post_call",
    ] {
        answer.insert(String::from(name), Value::Null);
    }
    answer.insert(
        String::from("supported"),
        json!(OpenAiImageVariationConfig::SUPPORTED_PARAMS),
    );
    answer.insert(String::from("calls"), Value::Array(Vec::new()));
    answer
}

pub(crate) fn transform_value(probe: &Probe) -> HelperResult {
    let data =
        OpenAiImageVariationConfig::request_data(image_for(probe), probe.optional_params.clone());
    let mut answer = empty_answer();
    if let Some(data) = data {
        let view = data_view(&data);
        answer.insert(
            String::from("data"),
            view.get("data").cloned().unwrap_or(Value::Null),
        );
    }
    Ok(Value::Object(answer))
}

pub(crate) fn mapped_value(probe: &Probe) -> HelperResult {
    let mut optional_params = probe.existing_params.clone();
    OpenAiImageVariationConfig::map_params(&probe.non_default_params, &mut optional_params);
    let mut answer = empty_answer();
    answer.insert(String::from("mapped"), json!(optional_params));
    Ok(Value::Object(answer))
}

pub(crate) fn request_value(probe: &Probe) -> HelperResult {
    let transport = FakeTransport::new(probe)?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| invalid(error.to_string()))?;
    let mut recorder = Recorder::default();
    let outcome = match begin_image_variation(call_for(probe), &mut recorder) {
        Ok(prepared) => {
            let sent = runtime.block_on(send_image_variation(&prepared, &transport));
            finish_image_variation(&prepared, sent, &mut recorder)
        }
        Err(error) => Err(error),
    };
    let calls = transport
        .calls
        .into_inner()
        .unwrap_or_else(PoisonError::into_inner);
    let (result, error) = match outcome {
        Ok(found) => (
            serde_json::to_value(&found.reply).map_err(|error| invalid(error.to_string()))?,
            Value::Null,
        ),
        Err(failure) => (Value::Null, failure_view(&failure)),
    };
    let mut answer = empty_answer();
    answer.insert(String::from("supplied"), json!(probe.client.is_some()));
    answer.insert(String::from("kind"), json!("sync"));
    answer.insert(String::from("calls"), Value::Array(calls));
    answer.insert(String::from("result"), result);
    answer.insert(String::from("error"), error);
    Ok(Value::Object(answer))
}
