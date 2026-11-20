use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_config::loader::ConfigLoader;
use betterlite_models::decisions::Json;
use betterlite_openai::api::transcription_contract::TranscriptionResponse;
use betterlite_provider::api::transcription_contract::AudioFile;
use betterlite_provider::api::transcription_http::error::TranscriptionHttpError;
use betterlite_provider::api::transcription_http::reply::TranscriptionReply;
use betterlite_provider::api::transcription_http::{TranscriptionCall, TranscriptionDeps};

use super::super::error::SdkHttpError;
use super::super::result::kinds::SdkResult;
use super::support::{Fixture, FixtureConfig, source};
use super::{Call, Failure, Outcome, ProviderConfig, Raised, View};

// The production handlers over the mock's base: the same logging state and pooled client a request would build
pub(super) async fn execute(
    call: &Call,
    api_base: &str,
    env: &BTreeMap<String, String>,
) -> Result<SdkResult, SdkHttpError> {
    let config = ConfigLoader {
        lookup: env.clone(),
    }
    .load("model_list: []")
    .map_err(|error| SdkHttpError::ConfigLoad(Box::new(error)))?;
    let origin = source(&config, env)?;
    let fixture_config = FixtureConfig { call };
    let configured = match call.provider_config {
        ProviderConfig::Present => Some(&fixture_config),
        ProviderConfig::Missing => None,
    };
    let audio_file = AudioFile {
        filename: "a.wav",
        content: b"abc",
        content_type: "audio/wav",
    };
    let request = TranscriptionCall {
        model: &call.model,
        audio_file,
        optional_params: &call.optional_params,
        litellm_params: &call.litellm_params,
        api_key: call.api_key.as_deref(),
        api_base: Some(api_base),
        custom_llm_provider: "openai",
        headers: call.headers.as_ref(),
        atranscription: Json::Bool(call.atranscription),
        timeout: call.timeout_seconds.map(Duration::from_secs),
        max_retries: call.max_retries,
        shared_session: None,
    };
    let deps = TranscriptionDeps {
        config: configured,
        source: &origin,
        supplied: None,
    };
    let mut fixture = Fixture::new()?;
    let answer = fixture.run(&deps, &request, call.mode).await;
    Ok(SdkResult::TranscriptionHttp {
        value: Box::new(outcome(call, answer)?),
    })
}

// A provider failure and a missing config are what both sides can observe; anything else is betterlite-only
fn outcome(
    call: &Call,
    answer: Result<TranscriptionReply, TranscriptionHttpError>,
) -> Result<Outcome, SdkHttpError> {
    match answer {
        Ok(reply) => Ok(Outcome {
            result: Some(view(call, reply)?),
            failure: None,
            raised: None,
        }),
        Err(TranscriptionHttpError::Provider(error)) => Ok(Outcome {
            result: None,
            failure: Some(Failure {
                status: error.status_code,
                synthesized: error.status_code_is_synthesized,
                message: error.message,
                headers: error.headers,
            }),
            raised: None,
        }),
        Err(error @ TranscriptionHttpError::MissingProviderConfig { .. }) => Ok(Outcome {
            result: None,
            failure: None,
            raised: Some(Raised {
                class: String::from("ValueError"),
                message: error.to_string(),
            }),
        }),
        Err(other) => Err(SdkHttpError::Invalid(other.to_string())),
    }
}

fn view(call: &Call, reply: TranscriptionReply) -> Result<View, SdkHttpError> {
    let TranscriptionReply {
        response: transformed,
        provider_headers,
    } = reply;
    let mut headers = BTreeMap::new();
    for name in &call.observe_headers {
        if let Some(value) = provider_headers.headers.get(name) {
            headers.insert(name.clone(), value.clone());
        }
    }
    Ok(View {
        text: transformed.text,
        words: words(&transformed)?,
        headers,
    })
}

fn words(transformed: &TranscriptionResponse) -> Result<Option<Json>, SdkHttpError> {
    transformed
        .extra
        .get("words")
        .cloned()
        .map(|value| serde_json::from_value(value).map_err(SdkHttpError::ResultParse))
        .transpose()
}
