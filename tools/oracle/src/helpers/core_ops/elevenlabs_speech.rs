use std::collections::BTreeMap;

use betterlite_config::types::LiteLlmParams;
use betterlite_core::api::elevenlabs::speech::auth::{SpeechKeys, validate_environment};
use betterlite_core::api::elevenlabs::speech::endpoint::complete_url;
use betterlite_core::api::elevenlabs::speech::mapping::map_openai_params;
use betterlite_core::api::elevenlabs::speech::request::transform_request;
use betterlite_openai::api::speech_provider_contract::SpeechVoice;
use betterlite_openai::openai_base_config::Object;
use indexmap::IndexMap;
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::{Value, json};

const MODEL: &str = "eleven_multilingual_v2";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    calls: Vec<Call>,
}

#[derive(Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
enum Call {
    MapParams {
        voice: Option<SpeechVoice>,
        optional_params: Object,
        kwargs: Object,
    },
    Request {
        text: String,
        optional_params: Object,
    },
    Url {
        api_base: Option<String>,
        environment: BTreeMap<String, String>,
        litellm_params: Object,
    },
    Headers {
        headers: IndexMap<String, String>,
        api_key: Option<String>,
        litellm_api_key: Option<String>,
        litellm_openai_key: Option<String>,
        environment: BTreeMap<String, String>,
    },
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let answers = input.calls.iter().map(answer).collect();
        Ok(json!({ "calls": Value::Array(answers) }))
    }
}

fn answer(call: &Call) -> Value {
    match call {
        Call::MapParams {
            voice,
            optional_params,
            kwargs,
        } => map_answer(voice.as_ref(), optional_params, kwargs),
        Call::Request {
            text,
            optional_params,
        } => request_answer(text, optional_params),
        Call::Url {
            api_base,
            environment,
            litellm_params,
        } => url_answer(api_base.as_deref(), environment, litellm_params),
        Call::Headers {
            headers,
            api_key,
            litellm_api_key,
            litellm_openai_key,
            environment,
        } => headers_answer(
            headers,
            [
                api_key.as_ref(),
                litellm_api_key.as_ref(),
                litellm_openai_key.as_ref(),
            ],
            environment,
        ),
    }
}

fn request_answer(text: &str, optional_params: &Object) -> Value {
    let data = transform_request(MODEL, text, optional_params);
    json!({ "ok": true, "dict_body": data.dict_body, "headers": data.headers })
}

fn url_answer(
    api_base: Option<&str>,
    environment: &BTreeMap<String, String>,
    litellm_params: &Object,
) -> Value {
    let (_, environment_base) = LiteLlmParams::elevenlabs_speech_settings(environment);
    complete_url(api_base, &environment_base, litellm_params).map_or_else(
        |_| json!({ "ok": false }),
        |url| json!({ "ok": true, "url": url }),
    )
}

fn map_answer(voice: Option<&SpeechVoice>, optional_params: &Object, kwargs: &Object) -> Value {
    let mut options = optional_params.clone();
    let mut forwarded = kwargs.clone();
    map_openai_params(&mut options, voice, &mut forwarded).map_or_else(
        |_| json!({ "ok": false }),
        |mapping| {
            json!({
                "ok": true,
                "voice_id": mapping.voice_id,
                "params": mapping.params,
                "optional_params": options,
                "kwargs": forwarded,
            })
        },
    )
}

fn headers_answer(
    headers: &IndexMap<String, String>,
    keys: [Option<&String>; 3],
    environment: &BTreeMap<String, String>,
) -> Value {
    let (environment_key, _) = LiteLlmParams::elevenlabs_speech_settings(environment);
    let [explicit, global_api_key, global_openai_key] =
        keys.map(|key| key.cloned().map(SecretString::from));
    let speech_keys = SpeechKeys {
        explicit: explicit.as_ref(),
        global_api_key: global_api_key.as_ref(),
        global_openai_key: global_openai_key.as_ref(),
        environment: environment_key.as_ref(),
    };
    validate_environment(headers, &speech_keys).map_or_else(
        |_| json!({ "ok": false }),
        |updated| json!({ "ok": true, "headers": updated }),
    )
}
