use std::collections::BTreeMap;

use betterlite_config::sections::Section as _;
use betterlite_config::sections::vertex_schema_shaping::{Raw, Settings};
use betterlite_prompts::arguments::json::Object;
use betterlite_vertex::gemini_image_policy::error::GeminiImagePolicyError;
use betterlite_vertex::gemini_image_policy::generation::{
    get_gemini_image_generation_config, get_gemini_image_web_search_requests,
};
use betterlite_vertex::gemini_image_policy::policy::{
    ImageParamMapping, is_gemini_image_model, map_openai_image_params_to_gemini,
    supports_gemini_image_size,
};
use betterlite_vertex::gemini_image_policy::tools::map_gemini_image_tools_params;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::helpers::{HelperError, HelperResult, OrHelperError as _};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub(super) settings: BTreeMap<String, String>,
    pub(super) cases: Vec<Case>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Case {
    model: String,
    catalog_supports_image_size: Option<bool>,
    params: Object,
    supported_params: Vec<String>,
    optional_params: Object,
    parse_image_config_string: bool,
    pre_mapped: Option<Object>,
    response: Object,
}

pub(super) fn run_cases(input: &Input) -> HelperResult {
    let settings = Raw::default().convert(&input.settings).or_config_error()?;
    let mut results = Vec::with_capacity(input.cases.len());
    for case in &input.cases {
        results.push(case_value(case, &settings)?);
    }
    Ok(json!({ "results": results }))
}

fn case_value(case: &Case, settings: &Settings) -> Result<Value, Box<HelperError>> {
    let supported_params: Vec<&str> = case.supported_params.iter().map(String::as_str).collect();
    let mapping = ImageParamMapping {
        params: &case.params,
        model: &case.model,
        supported_params: &supported_params,
        optional_params: &case.optional_params,
        image_size_support: case.catalog_supports_image_size,
        parse_image_config_string: case.parse_image_config_string,
    };
    let mut mapped = map_openai_image_params_to_gemini(&mapping).map_err(failure)?;
    if let Some(pre_mapped) = &case.pre_mapped {
        for (key, value) in pre_mapped {
            mapped.insert(key.clone(), value.clone());
        }
    }
    let tools = map_gemini_image_tools_params(&case.params, &mapped, settings).map_err(failure)?;
    Ok(json!({
        "supports_image_size": supports_gemini_image_size(&case.model, case.catalog_supports_image_size),
        "is_image_model": is_gemini_image_model(&case.model),
        "mapped": mapped,
        "tools": tools,
        "generation": get_gemini_image_generation_config(&case.model, &tools, case.catalog_supports_image_size),
        "web_search_requests": get_gemini_image_web_search_requests(&case.response),
    }))
}

fn failure(error: GeminiImagePolicyError) -> Box<HelperError> {
    let unsupported = matches!(error, GeminiImagePolicyError::ImageConfigJson { .. });
    Box::new(HelperError {
        class: String::from(if unsupported {
            "UnsupportedParamsError"
        } else {
            "ValueError"
        }),
        status: unsupported.then_some(400),
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}
