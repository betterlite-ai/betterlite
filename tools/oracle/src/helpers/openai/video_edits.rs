use betterlite_models::decisions::Json;
use betterlite_openai::video_edits::extension::{
    VideoExtensionRequest,
    // ast-grep-ignore: no-long-names
    transform_video_extension_request,
};
use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::{Value, json};

use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

use self::fixture::Fixture;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    calls: Vec<Call>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub(crate) enum Call {
    CreateCharacterRequest(CreateCharacterRequest),
    CreateCharacterResponse(CharacterResponse),
    GetCharacterRequest(GetCharacterRequest),
    GetCharacterResponse(CharacterResponse),
    EditRequest(EditRequest),
    EditResponse(EditResponse),
    ExtensionRequest(ExtensionRequest),
    ExtensionResponse(ExtensionResponse),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CreateCharacterRequest {
    name: String,
    fixture: Fixture,
    api_base: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct GetCharacterRequest {
    character_id: String,
    api_base: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EditRequest {
    prompt: String,
    video_id: String,
    api_base: String,
    #[serde(default)]
    fixture: Option<Fixture>,
    #[serde(default)]
    extra_body: Option<IndexMap<String, Json>>,
    #[serde(default)]
    prefetched_source_data: Option<Json>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ExtensionRequest {
    prompt: String,
    video_id: String,
    seconds: String,
    api_base: String,
    #[serde(default)]
    extra_body: Option<IndexMap<String, Json>>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CharacterResponse {
    body: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EditResponse {
    body: String,
    #[serde(default)]
    provider: Option<String>,
    #[serde(default)]
    request_data: Option<Json>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ExtensionResponse {
    body: String,
    #[serde(default)]
    provider: Option<String>,
}

fn run_call(call: &Call) -> HelperResult {
    match call {
        Call::CreateCharacterRequest(request) => requests::create_character_value(request),
        Call::CreateCharacterResponse(reply) | Call::GetCharacterResponse(reply) => {
            replies::character_reply(reply)
        }
        Call::GetCharacterRequest(request) => requests::get_character_value(request),
        Call::EditRequest(request) => requests::edit_request_value(request),
        Call::EditResponse(reply) => replies::video_reply_value(
            &reply.body,
            reply.provider.as_deref(),
            reply.request_data.as_ref(),
        ),
        Call::ExtensionRequest(request) => {
            let extension =
                // ast-grep-ignore: no-long-names
                transform_video_extension_request(VideoExtensionRequest {
                    prompt: request.prompt.clone(),
                    video_id: request.video_id.clone(),
                    seconds: request.seconds.clone(),
                    api_base: request.api_base.clone(),
                    extra_body: request.extra_body.clone(),
                });
            Ok(json!({"url": extension.url, "body": extension.body}))
        }
        Call::ExtensionResponse(reply) => {
            replies::video_reply_value(&reply.body, reply.provider.as_deref(), None)
        }
    }
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let mut results = Vec::with_capacity(input.calls.len());
        for call in &input.calls {
            results.push(run_call(call)?);
        }
        Ok(Value::Array(results))
    }
}

mod fixture;
mod replies;
mod requests;
mod support;
mod tests;
