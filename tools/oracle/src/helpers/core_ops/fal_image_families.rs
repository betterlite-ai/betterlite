use betterlite_core::api::fal::bytedance::{BytedanceModel, MAPPING_PROFILE};
use betterlite_core::api::fal::flux::{FluxImageResponse, project_response as project_flux};
use betterlite_core::api::fal::imagen::{
    ENDPOINT as IMAGEN_ENDPOINT, ImagenImageResponse, project_response as project_imagen,
};
use betterlite_core::api::fal::images::decode::{
    FalImage, append_generation_images, fal_images_to_image_objects,
};
use betterlite_core::api::fal::images::error::FalImageError;
use betterlite_core::api::fal::images::params::image_generation_body;
use betterlite_core::api::fal::images::settings::FalImageSettings;
use betterlite_core::api::fal::{imagen, nano_banana};
use betterlite_models::decisions::{HiddenParams, Json};
use betterlite_openai::api::image_contracts::records::{ImageHeaders, ImageParams};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

const MODEL: &str = "fal-ai/model";

#[derive(Debug, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    ImagenFixedPreview {
        prompt: String,
        models: Vec<String>,
        api_bases: Vec<Option<String>>,
        cases: Vec<MappingCase>,
    },
    RatioAlgorithmsDiffer {
        sizes: Vec<String>,
        banana_sizes: Vec<Json>,
    },
    BananaDestinationWins {
        prompt: String,
        models: Vec<String>,
        api_bases: Vec<Option<String>>,
        cases: Vec<MappingCase>,
    },
    BananaResolutionPassthrough {
        prompt: String,
        cases: Vec<MappingCase>,
    },
    BytedanceEndpoints {
        prompt: String,
        families: Vec<Family>,
        api_bases: Vec<Option<String>>,
        cases: Vec<MappingCase>,
    },
    FamilyReplyMetadata {
        families: Vec<Family>,
        replies: Vec<ReplyCase>,
    },
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Family {
    Imagen,
    NanoBanana,
    Seedream,
    Dreamina,
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

impl Family {
    fn mapped_params(self, case: &MappingCase) -> Option<ImageParams> {
        let optional = case.optional.clone();
        match self {
            Self::Imagen => {
                imagen::map_openai_params(&case.non_default, optional, MODEL, case.drop_params).ok()
            }
            Self::NanoBanana => {
                nano_banana::map_openai_params(&case.non_default, optional, MODEL, case.drop_params)
                    .ok()
            }
            Self::Seedream | Self::Dreamina => MAPPING_PROFILE
                .map_openai_params(&case.non_default, optional, MODEL, case.drop_params)
                .ok(),
        }
    }

    fn url(self, api_base: Option<&str>, model: &str) -> String {
        let endpoint = match self {
            Self::Imagen => IMAGEN_ENDPOINT.to_owned(),
            Self::NanoBanana => nano_banana::endpoint(model),
            Self::Seedream => BytedanceModel::SeedreamV3.endpoint().to_owned(),
            Self::Dreamina => BytedanceModel::DreaminaV31.endpoint().to_owned(),
        };
        FalImageSettings::default().complete_url(api_base, &endpoint)
    }

    fn reply(self, reply: &ReplyCase) -> Result<(Vec<FalImage>, HiddenParams), FalImageError> {
        let existing = fal_images_to_image_objects(&Json::List(reply.existing.clone()));
        let body = reply.body.as_bytes();
        match self {
            Self::Imagen => {
                let mut response = ImagenImageResponse {
                    data: existing,
                    hidden_params: HiddenParams::new(),
                };
                project_imagen(reply.status, &reply.headers, body, &mut response)?;
                Ok((response.data, response.hidden_params))
            }
            Self::NanoBanana => {
                let mut data = Some(existing);
                append_generation_images(&mut data, reply.status, &reply.headers, body)?;
                Ok((data.unwrap_or_default(), HiddenParams::new()))
            }
            Self::Seedream | Self::Dreamina => {
                let mut response = FluxImageResponse {
                    data: existing,
                    hidden_params: HiddenParams::new(),
                };
                project_flux(reply.status, &reply.headers, body, &mut response)?;
                Ok((response.data, response.hidden_params))
            }
        }
    }
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::ImagenFixedPreview {
                prompt,
                models,
                api_bases,
                cases,
            } => Ok(json!({
                "endpoint": IMAGEN_ENDPOINT,
                "urls": url_outcomes(Family::Imagen, models, api_bases),
                "cases": request_outcomes(Family::Imagen, prompt, cases),
            })),
            Input::RatioAlgorithmsDiffer {
                sizes,
                banana_sizes,
            } => Ok(ratio_outcomes(sizes, banana_sizes)),
            Input::BananaDestinationWins {
                prompt,
                models,
                api_bases,
                cases,
            } => Ok(json!({
                "urls": url_outcomes(Family::NanoBanana, models, api_bases),
                "cases": request_outcomes(Family::NanoBanana, prompt, cases),
            })),
            Input::BananaResolutionPassthrough { prompt, cases } => Ok(json!({
                "cases": request_outcomes(Family::NanoBanana, prompt, cases),
            })),
            Input::BytedanceEndpoints {
                prompt,
                families,
                api_bases,
                cases,
            } => Ok(bytedance_outcomes(prompt, families, api_bases, cases)),
            Input::FamilyReplyMetadata { families, replies } => {
                Ok(reply_outcomes(families, replies))
            }
        }
    }
}

fn url_outcomes(family: Family, models: &[String], api_bases: &[Option<String>]) -> Vec<Value> {
    let mut urls = Vec::new();
    for model in models {
        for api_base in api_bases {
            urls.push(json!({
                "model": model,
                "api_base": api_base,
                "url": family.url(api_base.as_deref(), model),
            }));
        }
    }
    urls
}

fn request_outcomes(family: Family, prompt: &str, cases: &[MappingCase]) -> Vec<Value> {
    cases
        .iter()
        .map(|case| {
            family.mapped_params(case).map_or_else(
                || json!({"error": {"status": null}}),
                |mapped| json!({"request": image_generation_body(prompt, mapped)}),
            )
        })
        .collect()
}

fn ratio_outcomes(sizes: &[String], banana_sizes: &[Json]) -> Value {
    let mut cases = Vec::new();
    for size in sizes {
        cases.push(size_outcome(Family::Imagen, &Json::Text(size.clone())));
        cases.push(size_outcome(Family::NanoBanana, &Json::Text(size.clone())));
    }
    for size in banana_sizes {
        cases.push(size_outcome(Family::NanoBanana, size));
    }
    json!({"cases": cases})
}

fn size_outcome(family: Family, size: &Json) -> Value {
    let mut non_default = ImageParams::new();
    non_default.insert("size".to_owned(), size.clone());
    let case = MappingCase {
        non_default,
        optional: ImageParams::new(),
        drop_params: false,
    };
    let outcome = family.mapped_params(&case).map_or_else(
        || json!({"error": {"status": null}}),
        |params| json!({"params": params}),
    );
    json!({"family": family, "size": size, "outcome": outcome})
}

fn bytedance_outcomes(
    prompt: &str,
    families: &[Family],
    api_bases: &[Option<String>],
    cases: &[MappingCase],
) -> Value {
    let entries: Vec<Value> = families
        .iter()
        .map(|family| {
            json!({
                "family": family,
                "urls": url_outcomes(*family, &[MODEL.to_owned()], api_bases),
                "cases": request_outcomes(*family, prompt, cases),
            })
        })
        .collect();
    json!({"families": entries})
}

fn reply_outcomes(families: &[Family], replies: &[ReplyCase]) -> Value {
    let entries: Vec<Value> = families
        .iter()
        .map(|family| {
            let outcomes: Vec<Value> = replies
                .iter()
                .map(|reply| reply_outcome(*family, reply))
                .collect();
            json!({"family": family, "replies": outcomes})
        })
        .collect();
    json!({"families": entries})
}

fn reply_outcome(family: Family, reply: &ReplyCase) -> Value {
    match family.reply(reply) {
        Ok((data, hidden_params)) => json!({"data": data, "hidden_params": hidden_params}),
        Err(
            FalImageError::ReplyTooLarge { status, .. }
            | FalImageError::ReplyDecode { status, .. }
            | FalImageError::ReplyNotAnObject { status, .. },
        ) => json!({"error": {"status": status}}),
        Err(
            FalImageError::MissingApiKey
            | FalImageError::UnsupportedParameters { .. }
            | FalImageError::SizeNotText
            | FalImageError::FieldNotText { .. },
        ) => json!({"error": "not a reply error"}),
    }
}
