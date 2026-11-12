use betterlite_models::decisions::Json;
use betterlite_openai::image_generation::family::{
    ImageFamily, ImageParamError, map_openai_params, select_image_family,
};
use betterlite_openai::image_generation::reply::ImageGenerationReply;
use betterlite_openai::image_generation::transform::{ImageTransformCall, transform_image_reply};
use indexmap::IndexMap;
use serde_json::{Value, json};

use super::input::Probe;
use super::support::{NOW, Recorder, invalid, observe};
use crate::helpers::{HelperError, HelperResult};

pub(crate) fn value(probe: &Probe) -> HelperResult {
    let family = select_image_family(&probe.model);
    let recorder = Recorder::default();
    let reply = transform(probe, family, &recorder)?;
    Ok(json!({
        "family": family_name(family),
        "supported": family.supported_params(),
        "transform": serde_json::to_value(&reply).map_err(|error| invalid(error.to_string()))?,
        "post_call": recorder.last("post_api_call"),
    }))
}

fn transform(
    probe: &Probe,
    family: ImageFamily,
    recorder: &Recorder,
) -> Result<ImageGenerationReply, Box<HelperError>> {
    let request_data = request_data(probe);
    let call = ImageTransformCall {
        family,
        raw: probe.reply.body.as_bytes(),
        optional_params: &probe.optional_params,
        request_data: &request_data,
        created_seconds: NOW,
    };
    observe(recorder, |state, env| {
        transform_image_reply(call, state, env)
    })?
    .map_err(|error| invalid(error.to_string()))
}

pub(crate) fn mapped_value(probe: &Probe) -> Value {
    let family = select_image_family(&probe.model);
    let mut mapped = probe.existing_params.clone();
    let outcome = map_openai_params(
        family,
        &probe.model,
        &probe.non_default_params,
        &mut mapped,
        probe.drop_params,
    );
    let error: Value = match outcome {
        Ok(()) => Value::Null,
        Err(ImageParamError::Unsupported { .. }) => json!({"kind": "unsupported_parameter"}),
    };
    json!({
        "family": family_name(family),
        "supported": family.supported_params(),
        "mapped": mapped,
        "error": error,
    })
}

fn request_data(probe: &Probe) -> IndexMap<String, Json> {
    let mut data = IndexMap::from([
        (String::from("model"), Json::Text(probe.model.clone())),
        (String::from("prompt"), Json::Text(probe.prompt.clone())),
    ]);
    data.extend(probe.optional_params.clone());
    data
}

const fn family_name(family: ImageFamily) -> &'static str {
    match family {
        ImageFamily::DallE2 => "dall_e_2",
        ImageFamily::DallE3 => "dall_e_3",
        ImageFamily::GptImage => "gpt_image",
    }
}
