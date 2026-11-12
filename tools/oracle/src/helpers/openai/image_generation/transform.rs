use betterlite_models::decisions::Json;
use betterlite_openai::image_generation::family::{
    ImageFamily, ImageParamError, map_openai_params, select_image_family,
};
use betterlite_openai::image_generation::transform::{ImageTransformCall, transform_image_reply};
use indexmap::IndexMap;
use serde_json::{Map, Value, json};

use super::Function;
use super::input::Probe;
use super::support::{NOW, Recorder, invalid, observe};
use crate::helpers::{HelperError, HelperResult};

type Detail = [(&'static str, Value); 2];

pub(crate) fn value(probe: &Probe, function: Function) -> HelperResult {
    let family = select_image_family(&probe.model);
    let detail = if function == Function::UnsupportedImageParam {
        mapped_detail(probe, family)
    } else {
        transform_detail(probe, family)?
    };
    let mut answer = Map::new();
    answer.insert(String::from("family"), json!(family_name(family)));
    answer.insert(String::from("supported"), json!(family.supported_params()));
    answer.extend(detail.map(|(name, found)| (String::from(name), found)));
    Ok(Value::Object(answer))
}

fn transform_detail(probe: &Probe, family: ImageFamily) -> Result<Detail, Box<HelperError>> {
    let recorder = Recorder::default();
    let request_data = request_data(probe);
    let call = ImageTransformCall {
        family,
        raw: probe.reply.body.as_bytes(),
        optional_params: &probe.optional_params,
        request_data: &request_data,
        created_seconds: NOW,
    };
    let reply = observe(&recorder, |state, env| {
        transform_image_reply(call, state, env)
    })?
    .map_err(|error| invalid(error.to_string()))?;
    Ok([
        (
            "transform",
            serde_json::to_value(&reply).map_err(|error| invalid(error.to_string()))?,
        ),
        ("post_call", json!(recorder.last("post_api_call"))),
    ])
}

fn mapped_detail(probe: &Probe, family: ImageFamily) -> Detail {
    let mut mapped = probe.existing_params.clone();
    let outcome = map_openai_params(
        family,
        &probe.model,
        &probe.non_default_params,
        &mut mapped,
        probe.drop_params,
    );
    let error = match outcome {
        Ok(()) => Value::Null,
        Err(ImageParamError::Unsupported { .. }) => json!({"kind": "unsupported_parameter"}),
    };
    [("mapped", json!(mapped)), ("error", error)]
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
