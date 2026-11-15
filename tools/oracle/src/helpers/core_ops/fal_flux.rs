use std::collections::BTreeMap;

use betterlite_config::sections::Sections;
use betterlite_core::api::fal::flux::{
    FluxImageResponse, FluxProfile, SUPPORTED_OPENAI_PARAMS, project_response,
};
use betterlite_core::api::fal::images::decode::{FalImage, fal_images_to_image_objects};
use betterlite_core::api::fal::images::params::image_generation_body;
use betterlite_core::api::fal::images::settings::FalImageSettings;
use betterlite_models::decisions::{HiddenParams, Json};
use betterlite_openai::api::image_contracts::records::{ImageHeaders, ImageParams};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::helpers::{Helper, HelperResult};

type Environment = BTreeMap<String, String>;

const MODEL: &str = "fal-ai/model";

#[derive(Debug, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    OriginalKeyPrecedence {
        prompt: String,
        profiles: Vec<Profile>,
        cases: Vec<MappingCase>,
    },
    ExactVsCustomRatio {
        profiles: Vec<Profile>,
        sizes: Vec<Json>,
    },
    PortraitOverlap {
        profiles: Vec<Profile>,
        sizes: Vec<Json>,
    },
    UnknownSizeFallback {
        profiles: Vec<Profile>,
        sizes: Vec<Json>,
        image_sizes: Vec<Json>,
    },
    DevEndpoint {
        environments: Vec<Environment>,
        api_bases: Vec<Option<String>>,
        profiles: Vec<Profile>,
    },
    HiddenMetadataPresence {
        profiles: Vec<Profile>,
        replies: Vec<ReplyCase>,
    },
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Profile {
    Ultra,
    ProV11,
    Schnell,
    Dev,
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
    existing: Vec<Json>,
}

impl Profile {
    const fn flux(self) -> FluxProfile {
        match self {
            Self::Ultra => FluxProfile::Ultra,
            Self::ProV11 => FluxProfile::ProV11,
            Self::Schnell => FluxProfile::Schnell,
            Self::Dev => FluxProfile::Dev,
        }
    }
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::OriginalKeyPrecedence {
                prompt,
                profiles,
                cases,
            } => Ok(precedence_outcomes(prompt, profiles, cases)),
            Input::ExactVsCustomRatio { profiles, sizes }
            | Input::PortraitOverlap { profiles, sizes } => Ok(size_outcomes(profiles, sizes)),
            Input::UnknownSizeFallback {
                profiles,
                sizes,
                image_sizes,
            } => Ok(fallback_outcomes(profiles, sizes, image_sizes)),
            Input::DevEndpoint {
                environments,
                api_bases,
                profiles,
            } => Ok(endpoint_outcomes(environments, api_bases, profiles)),
            Input::HiddenMetadataPresence { profiles, replies } => {
                Ok(hidden_outcomes(profiles, replies))
            }
        }
    }
}

fn precedence_outcomes(prompt: &str, profiles: &[Profile], cases: &[MappingCase]) -> Value {
    let entries: Vec<Value> = profiles
        .iter()
        .map(|profile| {
            let outcomes: Vec<Value> = cases
                .iter()
                .map(|case| precedence_outcome(profile.flux(), prompt, case))
                .collect();
            json!({
                "profile": profile,
                "supported": SUPPORTED_OPENAI_PARAMS,
                "cases": outcomes,
            })
        })
        .collect();
    json!({"profiles": entries})
}

fn precedence_outcome(profile: FluxProfile, prompt: &str, case: &MappingCase) -> Value {
    match profile.map_openai_params(
        &case.non_default,
        case.optional.clone(),
        MODEL,
        case.drop_params,
    ) {
        Ok(mapped) => json!({"request": image_generation_body(prompt, mapped)}),
        Err(_) => error_outcome(),
    }
}

fn size_outcomes(profiles: &[Profile], sizes: &[Json]) -> Value {
    let mut cases = Vec::new();
    for profile in profiles {
        for size in sizes {
            cases.push(size_outcome(*profile, size));
        }
    }
    json!({"cases": cases})
}

fn fallback_outcomes(profiles: &[Profile], sizes: &[Json], image_sizes: &[Json]) -> Value {
    let mut cases = Vec::new();
    for profile in profiles {
        for size in sizes {
            cases.push(size_outcome(*profile, size));
        }
        if !matches!(profile, Profile::Ultra) {
            for size in image_sizes {
                cases.push(size_outcome(*profile, size));
            }
        }
    }
    json!({"cases": cases})
}

fn size_outcome(profile: Profile, size: &Json) -> Value {
    let mut non_default = ImageParams::new();
    non_default.insert("size".to_owned(), size.clone());
    let mapped = profile
        .flux()
        .map_openai_params(&non_default, ImageParams::new(), MODEL, false);
    let outcome = match mapped {
        Ok(params) => json!({"params": params}),
        Err(_) => error_outcome(),
    };
    json!({"profile": profile, "size": size, "outcome": outcome})
}

fn endpoint_outcomes(
    environments: &[Environment],
    api_bases: &[Option<String>],
    profiles: &[Profile],
) -> Value {
    let entries: Vec<Value> = environments
        .iter()
        .map(|environment| {
            let (api_key, api_base) = Sections::fal_image_environment(environment);
            let settings = FalImageSettings { api_key, api_base };
            let outcomes: Vec<Value> = profiles
                .iter()
                .map(|profile| {
                    let endpoint = profile.flux().endpoint();
                    let urls: Vec<String> = api_bases
                        .iter()
                        .map(|base| settings.complete_url(base.as_deref(), endpoint))
                        .collect();
                    json!({"profile": profile, "endpoint": endpoint, "urls": urls})
                })
                .collect();
            json!({"environment": environment, "profiles": outcomes})
        })
        .collect();
    json!({"environments": entries})
}

fn hidden_outcomes(profiles: &[Profile], replies: &[ReplyCase]) -> Value {
    let entries: Vec<Value> = profiles
        .iter()
        .map(|profile| {
            let outcomes: Vec<Value> = replies.iter().map(hidden_outcome).collect();
            json!({"profile": profile, "replies": outcomes})
        })
        .collect();
    json!({"profiles": entries})
}

fn hidden_outcome(reply: &ReplyCase) -> Value {
    let mut response = FluxImageResponse {
        data: fal_images_to_image_objects(&Json::List(reply.existing.clone())),
        hidden_params: HiddenParams::new(),
    };
    match project_response(
        reply.status,
        &reply.headers,
        reply.body.as_bytes(),
        &mut response,
    ) {
        Ok(()) => json!({
            "data": records(&response.data),
            "hidden_params": response.hidden_params,
        }),
        Err(_) => json!({"error": {"status": reply.status}}),
    }
}

fn records(images: &[FalImage]) -> Vec<Value> {
    images.iter().map(record).collect()
}

fn record(image: &FalImage) -> Value {
    json!({
        "url": image.url.as_deref(),
        "b64_json": image.b64_json.as_deref(),
        "provider_specific_fields": image.provider_specific_fields.as_ref(),
    })
}

fn error_outcome() -> Value {
    json!({"error": {"status": null}})
}
