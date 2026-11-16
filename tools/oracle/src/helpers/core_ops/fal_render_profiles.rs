use std::collections::BTreeMap;

use betterlite_config::sections::Sections;
use betterlite_core::api::fal::ideogram::GENERATION_ENDPOINT;
use betterlite_core::api::fal::ideogram::params::map_openai_params as map_ideogram_params;
use betterlite_core::api::fal::ideogram::reply::transform_reply as transform_ideogram_reply;
use betterlite_core::api::fal::images::params::image_generation_body;
use betterlite_core::api::fal::images::settings::FalImageSettings;
use betterlite_core::api::fal::stable_diffusion::complete_url as stable_diffusion_url;
use betterlite_core::api::fal::stable_diffusion::params::map_openai_params as map_diffusion_params;
use betterlite_core::api::fal::stable_diffusion::reply::transform_reply as transform_diffusion_reply;
use betterlite_models::decisions::{HiddenParams, Json};
use betterlite_openai::api::image_contracts::records::{
    ImageData, ImageHeaders, ImageHttpResponse, ImageParams, ImageResponse,
};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::helpers::{Helper, HelperResult};

type Environment = BTreeMap<String, String>;

const IDEOGRAM_MODEL: &str = "fal-ai/ideogram/v3";
const DIFFUSION_MODEL: &str = "stable-diffusion-v35-medium";
const PROMPT: &str = "a prompt";

#[derive(Debug, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    WhitespaceSize {
        sizes: Vec<Json>,
    },
    SizeObjectExtraFields {
        sizes: Vec<Json>,
    },
    PortraitPresetDifference {
        sizes: Vec<Json>,
    },
    CustomDiffusionPath {
        environment: Environment,
        api_bases: Vec<Option<String>>,
        models: Vec<String>,
        model: String,
        prompt: String,
        cases: Vec<MappingCase>,
    },
    InvalidReplyMember {
        replies: Vec<ReplyCase>,
    },
    HiddenFieldsByFamily {
        replies: Vec<ReplyCase>,
    },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MappingCase {
    #[serde(default)]
    non_default: ImageParams,
    #[serde(default)]
    optional: ImageParams,
    #[serde(default)]
    drop_params: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReplyCase {
    status: u16,
    #[serde(default)]
    headers: ImageHeaders,
    body: String,
    #[serde(default)]
    data: Vec<ImageData>,
    #[serde(default)]
    hidden_params: HiddenParams,
}

#[derive(Clone, Copy)]
enum Family {
    Ideogram,
    Diffusion,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::WhitespaceSize { sizes }
            | Input::SizeObjectExtraFields { sizes }
            | Input::PortraitPresetDifference { sizes } => Ok(size_outcomes(sizes)),
            Input::CustomDiffusionPath {
                environment,
                api_bases,
                models,
                model,
                prompt,
                cases,
            } => {
                let (api_key, api_base) = Sections::fal_image_environment(environment);
                let settings = FalImageSettings { api_key, api_base };
                Ok(json!({
                    "urls": url_outcomes(&settings, api_bases, models),
                    "cases": case_outcomes(model, prompt, cases),
                }))
            }
            Input::InvalidReplyMember { replies } | Input::HiddenFieldsByFamily { replies } => {
                let cases: Vec<Value> = replies.iter().map(reply_outcome).collect();
                Ok(json!({"cases": cases}))
            }
        }
    }
}

fn size_outcomes(sizes: &[Json]) -> Value {
    let cases: Vec<Value> = sizes
        .iter()
        .map(|size| {
            let mut non_default = ImageParams::new();
            non_default.insert("size".to_owned(), size.clone());
            let case = MappingCase {
                non_default,
                optional: ImageParams::new(),
                drop_params: false,
            };
            json!({
                "ideogram": mapping_outcome(Family::Ideogram, &case, IDEOGRAM_MODEL, PROMPT),
                "stable_diffusion": mapping_outcome(Family::Diffusion, &case, DIFFUSION_MODEL, PROMPT),
            })
        })
        .collect();
    json!({"sizes": cases})
}

fn url_outcomes(
    settings: &FalImageSettings,
    api_bases: &[Option<String>],
    models: &[String],
) -> Vec<Value> {
    api_bases
        .iter()
        .flat_map(|api_base| {
            models
                .iter()
                .map(|model| url_pair(settings, api_base.as_deref(), model))
        })
        .collect()
}

fn url_pair(settings: &FalImageSettings, api_base: Option<&str>, model: &str) -> Value {
    json!({
        "ideogram": settings.complete_url(api_base, GENERATION_ENDPOINT),
        "stable_diffusion": stable_diffusion_url(settings, api_base, model),
    })
}

fn case_outcomes(model: &str, prompt: &str, cases: &[MappingCase]) -> Vec<Value> {
    cases
        .iter()
        .map(|case| {
            json!({
                "ideogram": mapping_outcome(Family::Ideogram, case, model, prompt),
                "stable_diffusion": mapping_outcome(Family::Diffusion, case, model, prompt),
            })
        })
        .collect()
}

fn mapping_outcome(family: Family, case: &MappingCase, model: &str, prompt: &str) -> Value {
    match mapped_params(family, case, model) {
        Ok(params) => mapped_body(&params, prompt),
        Err(status) => json!({"error": {"status": status}}),
    }
}

fn mapped_params(
    family: Family,
    case: &MappingCase,
    model: &str,
) -> Result<ImageParams, Option<u16>> {
    match family {
        Family::Ideogram => map_ideogram_params(
            &case.non_default,
            case.optional.clone(),
            model,
            case.drop_params,
        )
        .map_err(|error| error.status()),
        Family::Diffusion => map_diffusion_params(
            &case.non_default,
            case.optional.clone(),
            model,
            case.drop_params,
        )
        .map_err(|error| error.status()),
    }
}

fn mapped_body(params: &ImageParams, prompt: &str) -> Value {
    let body = image_generation_body(prompt, params.clone());
    json!({
        "params": params,
        "body": body,
    })
}

fn reply_outcome(case: &ReplyCase) -> Value {
    json!({
        "ideogram": reply_family_outcome(Family::Ideogram, case),
        "stable_diffusion": reply_family_outcome(Family::Diffusion, case),
    })
}

fn reply_family_outcome(family: Family, case: &ReplyCase) -> Value {
    let mut response = ImageResponse {
        created: 1,
        data: Some(case.data.clone()),
        usage: None,
    };
    let mut hidden_params = case.hidden_params.clone();
    match transformed_reply(family, case, &mut response, &mut hidden_params) {
        Ok(()) => success_outcome(&response, &hidden_params),
        Err(status) => json!({"error": {"status": status}}),
    }
}

fn transformed_reply(
    family: Family,
    case: &ReplyCase,
    response: &mut ImageResponse,
    hidden_params: &mut HiddenParams,
) -> Result<(), Option<u16>> {
    let reply = http_response(case);
    match family {
        Family::Ideogram => transform_ideogram_reply(response, hidden_params, &reply)
            .map_err(|error| error.status()),
        Family::Diffusion => transform_diffusion_reply(response, hidden_params, &reply)
            .map_err(|error| error.status()),
    }
}

fn http_response(case: &ReplyCase) -> ImageHttpResponse {
    ImageHttpResponse {
        status_code: case.status,
        headers: case.headers.clone(),
        body: case.body.as_bytes().to_vec(),
    }
}

fn success_outcome(response: &ImageResponse, hidden_params: &HiddenParams) -> Value {
    json!({
        "data": images_value(response),
        "hidden_params": hidden_params,
    })
}

fn images_value(response: &ImageResponse) -> Vec<Value> {
    response
        .data
        .as_deref()
        .unwrap_or_default()
        .iter()
        .map(|image| {
            json!({
                "b64_json": image.b64_json.as_deref(),
                "url": image.url.as_deref(),
                "revised_prompt": image.revised_prompt.as_deref(),
            })
        })
        .collect()
}
