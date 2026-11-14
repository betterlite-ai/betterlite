use betterlite_config::sections::studio_identity::Settings;
use betterlite_models::decisions::Json;
use betterlite_vertex::google_count_tokens::cleaner::clean_contents;
use betterlite_vertex::google_count_tokens::studio::{StudioCountInput, studio_request};
use betterlite_vertex::studio_identity::NativeParams;
use serde::Deserialize;
use serde_json::json;

use crate::helpers::OrHelperError as _;

pub(crate) struct Op;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    calls: Vec<Call>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Call {
    model: String,
    contents: Json,
    api_base: Option<String>,
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let calls = input
            .calls
            .iter()
            .map(resolve)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(json!({ "calls": calls }))
    }
}

fn resolve(call: &Call) -> crate::helpers::HelperResult {
    let request = studio_request(&StudioCountInput {
        model: &call.model,
        contents: &call.contents,
        api_key: None,
        api_base: call.api_base.as_deref(),
        params: &NativeParams {
            api_key: None,
            gemini_api_key: None,
        },
        settings: &Settings::default(),
        timeout: None,
    })
    .or_value_error()?;
    let cleaned = clean_contents(&call.contents).or_value_error()?;
    Ok(json!({
        "contents": cleaned,
        "original": call.contents,
        "url": request.url,
    }))
}

mod tests;
