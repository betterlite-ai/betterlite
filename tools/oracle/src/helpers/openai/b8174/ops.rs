use betterlite_openai::minimax_speech::error::MinimaxSpeechError;
use betterlite_openai::minimax_speech::params::{MappedOptions, VoiceInput, map_openai_params};
use betterlite_openai::minimax_speech::reply::{SpeechReply, decode_speech_reply};
use betterlite_openai::minimax_speech::request::{
    get_complete_url, speech_request_body, validate_environment,
};
use betterlite_openai::modelscope_image::error::ModelScopeImageError;
use betterlite_openai::modelscope_image::params::map_openai_params as map_image_params;
use betterlite_openai::modelscope_image::reply::{ImageReply, decode_image_generation_reply};
use betterlite_openai::modelscope_image::request::{
    get_complete_url as image_url, image_request_body,
};
use betterlite_openai::openai_base_config::{Headers, Object};
use betterlite_openai::speech::result::SpeechResponse;
use http::header::{HeaderMap, HeaderName, HeaderValue};
use secrecy::SecretString;
use serde_json::{Value, json};

use betterlite_oracle_helper_api::error::HelperError;

use super::input::{SpeechHeaderKeys, Step, Voice};
use betterlite_openai::image_generation::reply::ImageDatum;
use tokio::runtime::Builder;

const BODY_LIMIT: usize = 1024 * 1024;

// LiteLLM raises here and the call ends, so a failed step ends the run with its status
fn failure(class: &str, status: Option<u16>, message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: class.to_owned(),
        status,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

trait ProviderFailure {
    fn helper_error(self) -> Box<HelperError>;
}

impl ProviderFailure for MinimaxSpeechError {
    fn helper_error(self) -> Box<HelperError> {
        failure("MinimaxException", self.status_code(), self.to_string())
    }
}

impl ProviderFailure for ModelScopeImageError {
    fn helper_error(self) -> Box<HelperError> {
        failure("ModelScopeImageError", self.status_code(), self.to_string())
    }
}

fn mapped_options(
    voice: Option<&Voice>,
    params: &Object,
) -> Result<MappedOptions, Box<HelperError>> {
    let input = voice.map(Voice::input);
    map_openai_params(input.as_ref(), params).map_err(|error| {
        failure(
            "UnsupportedParamsError",
            error.status_code(),
            error.to_string(),
        )
    })
}

fn header_map(headers: &Headers) -> Result<HeaderMap, Box<HelperError>> {
    let mut map = HeaderMap::new();
    for (name, value) in headers {
        let header_name = HeaderName::from_bytes(name.as_bytes())
            .map_err(|error| failure("InvalidInput", None, error.to_string()))?;
        let header_value = HeaderValue::from_str(value)
            .map_err(|error| failure("InvalidInput", None, error.to_string()))?;
        map.insert(header_name, header_value);
    }
    Ok(map)
}

fn speech_params(voice: Option<&Voice>, params: &Object) -> Result<Value, Box<HelperError>> {
    let mapped = mapped_options(voice, params)?;
    Ok(json!({
        "voice": mapped.voice,
        "params": mapped.params,
    }))
}

fn speech_body(
    voice: Option<&Voice>,
    model: &str,
    input: &str,
    params: &Object,
) -> Result<Value, Box<HelperError>> {
    let mapped = mapped_options(voice, params)?;
    Ok(Value::Object(speech_request_body(
        model,
        input,
        &mapped.voice,
        &mapped.params,
    )))
}

fn speech_headers(keys: &SpeechHeaderKeys) -> Result<Value, Box<HelperError>> {
    let key = keys.api.clone().map(SecretString::from);
    let global = keys.global.clone().map(SecretString::from);
    let environment = keys.environment.clone().map(SecretString::from);
    let headers = validate_environment(
        &Headers::new(),
        key.as_ref(),
        global.as_ref(),
        environment.as_ref(),
    )
    .map_err(|error| failure("ValueError", error.status_code(), error.to_string()))?;
    Ok(serde_json::to_value(headers)?)
}

fn render_audio(response: SpeechResponse) -> Result<Value, Box<HelperError>> {
    let status = response.status;
    let content_type = response.content_type.clone();
    let headers = response.headers.clone();
    let runtime = Builder::new_current_thread()
        .build()
        .map_err(|error| failure("InvalidInput", None, error.to_string()))?;
    let audio = runtime
        .block_on(response.content(BODY_LIMIT))
        .map_err(|error| failure("InvalidInput", None, error.to_string()))?;
    Ok(json!({
        "status": status,
        "content_type": content_type,
        "headers": headers,
        "body": hex::encode(audio),
    }))
}

fn speech_reply(status: u16, headers: &Headers, body: &str) -> Result<Value, Box<HelperError>> {
    let map = header_map(headers)?;
    let reply = SpeechReply {
        status,
        headers: &map,
        body: body.as_bytes(),
    };
    match decode_speech_reply(&reply) {
        Ok(response) => render_audio(response),
        Err(error) => Err(error.helper_error()),
    }
}

fn image_params(
    model: &str,
    params: &Object,
    drop_params: bool,
) -> Result<Value, Box<HelperError>> {
    let mut optional = Object::new();
    map_image_params(params, &mut optional, model, drop_params).map_err(|error| {
        failure(
            "UnsupportedOptionError",
            error.status_code(),
            error.to_string(),
        )
    })?;
    Ok(json!({"params": optional}))
}

fn image_reply(status: u16, body: &str, created: i64) -> Result<Value, Box<HelperError>> {
    let reply = decode_image_generation_reply(
        &ImageReply {
            status,
            body: body.as_bytes(),
        },
        created,
    )
    .map_err(ProviderFailure::helper_error)?;
    let data = reply.data.iter().map(datum).collect::<Vec<Value>>();
    let mut payload = Object::new();
    payload.insert(String::from("created"), Value::from(reply.created));
    payload.insert(String::from("data"), Value::Array(data));
    Ok(Value::Object(payload))
}

fn datum(image: &ImageDatum) -> Value {
    let mut fields = Object::new();
    if let Some(url) = &image.url {
        fields.insert(String::from("url"), Value::from(url.clone()));
    }
    if let Some(b64_json) = &image.b64_json {
        fields.insert(String::from("b64_json"), Value::from(b64_json.clone()));
    }
    if let Some(prompt) = &image.revised_prompt {
        fields.insert(String::from("revised_prompt"), Value::from(prompt.clone()));
    }
    Value::Object(fields)
}

impl Voice {
    fn input(&self) -> VoiceInput<'_> {
        match self {
            Self::Text(text) => VoiceInput::Text(text),
            Self::Fields(fields) => VoiceInput::Fields(fields),
        }
    }
}

pub(crate) fn execute(step: &Step) -> Result<Value, Box<HelperError>> {
    match step {
        Step::SpeechParams { voice, params } => speech_params(voice.as_ref(), params),
        Step::SpeechBody {
            model,
            input,
            voice,
            params,
        } => speech_body(voice.as_ref(), model, input, params),
        Step::SpeechUrl { api_base, env_base } => Ok(Value::from(get_complete_url(
            api_base.as_deref(),
            env_base.as_deref(),
        ))),
        Step::SpeechHeaders(keys) => speech_headers(keys),
        Step::SpeechReply {
            status,
            headers,
            body,
        } => speech_reply(*status, headers, body),
        Step::ImageParams {
            model,
            params,
            drop_params,
        } => image_params(model, params, *drop_params),
        Step::ImageUrl { api_base, env_base } => Ok(Value::from(image_url(
            api_base.as_deref(),
            env_base.as_deref(),
        ))),
        Step::ImageBody {
            model,
            prompt,
            params,
        } => Ok(Value::Object(image_request_body(model, prompt, params))),
        Step::ImageReply {
            status,
            body,
            created,
        } => image_reply(*status, body, *created),
    }
}
