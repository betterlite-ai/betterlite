use std::collections::BTreeMap;

use betterlite_config::sections::Sections;
use betterlite_core::api::fal::video_prepare::VideoHeaders;
use betterlite_core::api::fal::video_prepare::VideoParams;
use betterlite_core::api::fal::video_prepare::create::{CreateResponseInput, create_response};
use betterlite_core::api::fal::video_prepare::decode::decode_fal_video_reply;
use betterlite_core::api::fal::video_prepare::error::get_error_class;
use betterlite_core::api::fal::video_prepare::params::map_openai_params;
use betterlite_core::api::fal::video_prepare::settings::{FalVideoSettings, complete_url};
use betterlite_core::api::fal::video_prepare::urls::job_urls;
use betterlite_models::decisions::Json;
use betterlite_openai::openai_video_identifiers::ids::encode_video_id_with_provider;
use betterlite_openai::videos::reply::VideoObject;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::{Value, json};

mod tests;

type Environment = BTreeMap<String, String>;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ErrorCase {
    message: String,
    status: u16,
    #[serde(default)]
    headers: VideoHeaders,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CreateCase {
    body: String,
    #[serde(default)]
    request_data: VideoParams,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct KeyCase {
    #[serde(default)]
    api_key: Option<String>,
    #[serde(default)]
    litellm_api_key: Option<String>,
    #[serde(default)]
    headers: VideoHeaders,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct JobCase {
    model: String,
    request_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    H3AutoDuration {
        models: Vec<String>,
        seconds: Vec<Json>,
    },
    SeedanceDecimalDuration {
        models: Vec<String>,
        seconds: Vec<Json>,
    },
    ResolutionShortSide {
        models: Vec<String>,
        sizes: Vec<Json>,
    },
    ProviderOverride {
        model: String,
        now: i64,
        providers: Vec<Option<String>>,
        cases: Vec<CreateCase>,
        errors: Vec<ErrorCase>,
    },
    ReferenceType {
        models: Vec<String>,
        references: Vec<Json>,
    },
    QueueBaseLiteral {
        environments: Vec<Environment>,
        api_bases: Vec<Option<String>>,
        cases: Vec<KeyCase>,
    },
    WorkflowJobPath {
        api_bases: Vec<Option<String>>,
        jobs: Vec<JobCase>,
    },
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::H3AutoDuration { models, seconds }
            | Input::SeedanceDecimalDuration { models, seconds } => {
                Ok(mapped_report(models, "seconds", seconds))
            }
            Input::ResolutionShortSide { models, sizes } => {
                Ok(mapped_report(models, "size", sizes))
            }
            Input::ReferenceType { models, references } => {
                Ok(mapped_report(models, "input_reference", references))
            }
            Input::ProviderOverride {
                model,
                now,
                providers,
                cases,
                errors,
            } => Ok(provider_report(model, *now, providers, cases, errors)),
            Input::QueueBaseLiteral {
                environments,
                api_bases,
                cases,
            } => Ok(queue_report(environments, api_bases, cases)),
            Input::WorkflowJobPath { api_bases, jobs } => Ok(job_report(api_bases, jobs)),
        }
    }
}

fn mapped_report(models: &[String], key: &str, values: &[Json]) -> Value {
    let results: Vec<Value> = models
        .iter()
        .map(|model| {
            let cases: Vec<Value> = values
                .iter()
                .map(|value| mapped_case(model, key, value))
                .collect();
            json!({"model": model, "cases": cases})
        })
        .collect();
    json!({"results": results})
}

fn mapped_case(model: &str, key: &str, value: &Json) -> Value {
    let mut params = VideoParams::new();
    params.insert(key.to_owned(), value.clone());
    map_openai_params(&params, model, false).map_or_else(
        |_| json!({"error": {"status": null}}),
        |mapped| json!({"params": mapped}),
    )
}

fn provider_report(
    model: &str,
    now: i64,
    providers: &[Option<String>],
    cases: &[CreateCase],
    errors: &[ErrorCase],
) -> Value {
    let mut videos = Vec::new();
    for provider in providers {
        for case in cases {
            videos.push(create_outcome(model, now, provider.as_deref(), case));
        }
    }
    let error_values: Vec<Value> = errors.iter().map(error_fields).collect();
    json!({"videos": videos, "errors": error_values})
}

fn create_outcome(model: &str, now: i64, provider: Option<&str>, case: &CreateCase) -> Value {
    let headers = VideoHeaders::new();
    let Ok(reply) = decode_fal_video_reply(200, &headers, case.body.as_bytes()) else {
        return json!({"error": {"status": null}});
    };
    create_response(&CreateResponseInput {
        model,
        reply: &reply,
        custom_llm_provider: provider,
        request_data: &case.request_data,
        created_at: now,
    })
    .map_or_else(
        |_| json!({"error": {"status": null}}),
        |video| video_fields(&video),
    )
}

fn video_fields(video: &VideoObject) -> Value {
    json!({
        "id": video.id,
        "object": video.object,
        "status": video.status,
        "created_at": video.created_at,
        "completed_at": video.completed_at,
        "expires_at": video.expires_at,
        "error": video.error,
        "progress": video.progress,
        "remixed_from_video_id": video.remixed_from_video_id,
        "seconds": video.seconds,
        "size": video.size,
        "model": video.model,
        "usage": video.usage,
    })
}

fn error_fields(case: &ErrorCase) -> Value {
    let error = get_error_class(&case.message, case.status, &case.headers);
    json!({
        "status_code": error.status_code,
        "message": error.message,
    })
}

fn queue_report(
    environments: &[Environment],
    api_bases: &[Option<String>],
    cases: &[KeyCase],
) -> Value {
    let reports: Vec<Value> = environments
        .iter()
        .map(|environment| queue_environment(environment, api_bases, cases))
        .collect();
    json!({"environments": reports})
}

fn queue_environment(
    environment: &Environment,
    api_bases: &[Option<String>],
    cases: &[KeyCase],
) -> Value {
    let (api_key, _api_base) = Sections::fal_image_environment(environment);
    let settings = FalVideoSettings { api_key };
    let urls: Vec<String> = api_bases
        .iter()
        .map(|api_base| complete_url(api_base.as_deref()))
        .collect();
    let mut headers = Vec::new();
    for case in cases {
        headers.push(key_outcome(&settings, case));
    }
    json!({"urls": urls, "headers": headers})
}

fn key_outcome(settings: &FalVideoSettings, case: &KeyCase) -> Value {
    let explicit = case.api_key.as_deref().map(SecretString::from);
    let param = case.litellm_api_key.as_deref().map(SecretString::from);
    settings
        .authorize(&case.headers, explicit.as_ref(), param.as_ref())
        .map_or_else(
            |_| json!({"error": {"status": null}}),
            |headers| json!({"headers": headers}),
        )
}

fn job_report(api_bases: &[Option<String>], jobs: &[JobCase]) -> Value {
    let mut outcomes = Vec::new();
    for api_base in api_bases {
        let base = complete_url(api_base.as_deref());
        for job in jobs {
            outcomes.push(job_outcome(&base, job));
        }
    }
    json!({"jobs": outcomes})
}

fn job_outcome(base: &str, job: &JobCase) -> Value {
    let video_id =
        encode_video_id_with_provider(&job.request_id, "fal_ai", Some(job.model.as_str()));
    job_urls(&video_id, base).map_or_else(
        |_| {
            json!({
                "status": {"error": {"status": null}},
                "content": {"error": {"status": null}},
            })
        },
        |urls| {
            json!({
                "status": {"url": urls.status},
                "content": {"url": urls.content},
            })
        },
    )
}
