use std::collections::BTreeMap;

use betterlite_config::sections::Sections;
use betterlite_core::api::fal::bria;
use betterlite_core::api::fal::images::UrlOnlyImageError;
use betterlite_core::api::fal::images::params::{GENERIC_OPENAI_PARAMS, image_generation_body};
use betterlite_core::api::fal::images::settings::FalImageSettings;
use betterlite_core::api::fal::recraft;
use betterlite_models::decisions::Json;
use betterlite_openai::api::image_contracts::records::{
    ImageData, ImageHeaders, ImageParams, ImageResponse,
};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::helpers::{Helper, HelperResult};

type Environment = BTreeMap<String, String>;

#[derive(Debug, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    IgnoredCount {
        environment: Environment,
        api_bases: Vec<Option<String>>,
        cases: Vec<MappingCase>,
        bodies: Vec<BodyCase>,
    },
    BriaOverlapEdges {
        sizes: Vec<String>,
    },
    RecraftPresetDimensions {
        sizes: Vec<String>,
    },
    SingularVsPlural {
        bria: Vec<ReplyCase>,
        recraft: Vec<ReplyCase>,
    },
    EmptyImageMap {
        bria: Vec<ReplyCase>,
        recraft: Vec<ReplyCase>,
    },
    DiscardedReplyMetadata {
        bria: Vec<ReplyCase>,
        recraft: Vec<ReplyCase>,
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
pub(crate) struct BodyCase {
    prompt: String,
    #[serde(default)]
    provider_options: ImageParams,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReplyCase {
    status: u16,
    #[serde(default)]
    headers: ImageHeaders,
    body: String,
    #[serde(default)]
    existing: Vec<String>,
}

type MapParams =
    fn(&ImageParams, ImageParams, &str, bool) -> Result<ImageParams, UrlOnlyImageError>;

type AppendImages =
    fn(&mut ImageResponse, u16, &ImageHeaders, &[u8]) -> Result<(), UrlOnlyImageError>;

#[derive(Clone, Copy)]
struct Profile {
    endpoint: &'static str,
    map: MapParams,
}

const BRIA: Profile = Profile {
    endpoint: bria::ENDPOINT,
    map: bria::map_openai_params,
};

const RECRAFT: Profile = Profile {
    endpoint: recraft::ENDPOINT,
    map: recraft::map_openai_params,
};

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::IgnoredCount {
                environment,
                api_bases,
                cases,
                bodies,
            } => Ok(ignored_count_outcome(environment, api_bases, cases, bodies)),
            Input::BriaOverlapEdges { sizes } => Ok(size_outcomes(BRIA, sizes)),
            Input::RecraftPresetDimensions { sizes } => Ok(size_outcomes(RECRAFT, sizes)),
            Input::SingularVsPlural { bria, recraft }
            | Input::EmptyImageMap { bria, recraft }
            | Input::DiscardedReplyMetadata { bria, recraft } => Ok(reply_pair(bria, recraft)),
        }
    }
}

fn ignored_count_outcome(
    environment: &Environment,
    api_bases: &[Option<String>],
    cases: &[MappingCase],
    bodies: &[BodyCase],
) -> Value {
    let (api_key, api_base) = Sections::fal_image_environment(environment);
    let settings = FalImageSettings { api_key, api_base };
    json!({
        "bria": model_report(BRIA, &settings, api_bases, cases, bodies),
        "recraft": model_report(RECRAFT, &settings, api_bases, cases, bodies),
    })
}

fn model_report(
    profile: Profile,
    settings: &FalImageSettings,
    api_bases: &[Option<String>],
    cases: &[MappingCase],
    bodies: &[BodyCase],
) -> Value {
    let urls: Vec<String> = api_bases
        .iter()
        .map(|explicit| settings.complete_url(explicit.as_deref(), profile.endpoint))
        .collect();
    let outcomes: Vec<Value> = cases
        .iter()
        .map(|case| {
            mapping_outcome((profile.map)(
                &case.non_default,
                case.optional.clone(),
                profile.endpoint,
                case.drop_params,
            ))
        })
        .collect();
    let body_values: Vec<Value> = bodies
        .iter()
        .map(|body| {
            json!(image_generation_body(
                &body.prompt,
                body.provider_options.clone()
            ))
        })
        .collect();
    json!({
        "supported": GENERIC_OPENAI_PARAMS,
        "urls": urls,
        "cases": outcomes,
        "bodies": body_values,
    })
}

fn size_outcomes(profile: Profile, sizes: &[String]) -> Value {
    let outcomes: Vec<Value> = sizes
        .iter()
        .map(|size| {
            let mut non_default = ImageParams::new();
            non_default.insert("size".to_owned(), Json::Text(size.clone()));
            mapping_outcome((profile.map)(
                &non_default,
                ImageParams::new(),
                profile.endpoint,
                false,
            ))
        })
        .collect();
    json!({"sizes": outcomes})
}

fn mapping_outcome(mapped: Result<ImageParams, UrlOnlyImageError>) -> Value {
    mapped.map_or_else(
        |_| json!({"error": {"status": null}}),
        |params| json!({"params": params}),
    )
}

fn reply_pair(bria_cases: &[ReplyCase], recraft_cases: &[ReplyCase]) -> Value {
    let bria_images: Vec<Value> = bria_cases
        .iter()
        .map(|case| reply_outcome(case, bria::append_singular_image))
        .collect();
    let recraft_images: Vec<Value> = recraft_cases
        .iter()
        .map(|case| reply_outcome(case, recraft::append_url_images))
        .collect();
    json!({"bria": bria_images, "recraft": recraft_images})
}

fn reply_outcome(case: &ReplyCase, append: AppendImages) -> Value {
    let mut response = ImageResponse {
        created: 0,
        data: Some(
            case.existing
                .iter()
                .map(|url| ImageData {
                    b64_json: None,
                    revised_prompt: None,
                    url: Some(url.clone()),
                })
                .collect(),
        ),
        usage: None,
    };
    match append(
        &mut response,
        case.status,
        &case.headers,
        case.body.as_bytes(),
    ) {
        Ok(()) => json!({"images": image_fields(&response)}),
        Err(_) => json!({"error": {"status": case.status}}),
    }
}

fn image_fields(response: &ImageResponse) -> Vec<Value> {
    response
        .data
        .as_deref()
        .unwrap_or_default()
        .iter()
        .map(|image| {
            json!({
                "url": image.url.as_deref(),
                "b64_json": image.b64_json.as_deref(),
                "revised_prompt": image.revised_prompt.as_deref(),
            })
        })
        .collect()
}
