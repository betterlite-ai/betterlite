use betterlite_openai::minimax_speech::params::{VoiceInput, map_openai_params};
use betterlite_openai::minimax_speech::reply::{SpeechReply, decode_speech_reply};
use betterlite_openai::minimax_speech::request::{
    get_complete_url, speech_request_body, validate_environment,
};
use betterlite_openai::modelscope_image::params::map_openai_params as map_image_params;
use betterlite_openai::modelscope_image::reply::{ImageReply, decode_image_generation_reply};
use betterlite_openai::modelscope_image::request::{
    get_complete_url as image_url, image_request_body, validate_environment as image_environment,
};
use betterlite_openai::openai_base_config::{Headers, Object};
use betterlite_openai::speech::result::SpeechResponse;
use http::header::{HeaderMap, HeaderName, HeaderValue};
use secrecy::SecretString;
use serde_json::{Value, json};

use crate::helpers::HelperError;

use super::input::{Step, Voice};

const BODY_LIMIT: usize = 1024 * 1024;

fn invalid(message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: String::from("InvalidInput"),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

fn header_map(headers: &Headers) -> Result<HeaderMap, Box<HelperError>> {
    let mut map = HeaderMap::new();
    for (name, value) in headers {
        let header_name =
            HeaderName::from_bytes(name.as_bytes()).map_err(|error| invalid(error.to_string()))?;
        let header_value =
            HeaderValue::from_str(value).map_err(|error| invalid(error.to_string()))?;
        map.insert(header_name, header_value);
    }
    Ok(map)
}

fn speech_params(voice: Option<&Voice>, params: &Object) -> Result<Value, Box<HelperError>> {
    let voice = voice.map(Voice::input);
    match map_openai_params(voice, params) {
        Ok(mapped) => Ok(json!({"voice": mapped.voice, "params": mapped.params})),
        Err(error) => Ok(json!({"ok": false, "status": error.status_code()})),
    }
}

fn speech_body(
    voice: Option<&Voice>,
    model: &str,
    input: &str,
    params: &Object,
) -> Result<Value, Box<HelperError>> {
    let mapped = match map_openai_params(voice.map(Voice::input), params) {
        Ok(mapped) => mapped,
        Err(error) => return Ok(json!({"ok": false, "status": error.status_code()})),
    };
    Ok(Value::Object(speech_request_body(
        model,
        input,
        &mapped.voice,
        &mapped.params,
    )))
}

fn speech_headers(
    api_key: Option<&str>,
    global_key: Option<&str>,
    environment_key: Option<&str>,
) -> Result<Value, Box<HelperError>> {
    let key = api_key.map(|text| SecretString::from(text.to_owned()));
    let global = global_key.map(|text| SecretString::from(text.to_owned()));
    let environment = environment_key.map(|text| SecretString::from(text.to_owned()));
    match validate_environment(
        &Headers::new(),
        key.as_ref(),
        global.as_ref(),
        environment.as_ref(),
    ) {
        Ok(headers) => Ok(serde_json::to_value(headers)?),
        Err(error) => Ok(json!({"ok": false, "status": error.status_code()})),
    }
}

fn render_audio(response: SpeechResponse) -> Result<Value, Box<HelperError>> {
    let status = response.status;
    let content_type = response.content_type.clone();
    let headers = response.headers.clone();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .map_err(|error| invalid(error.to_string()))?;
    let audio = runtime
        .block_on(response.content(BODY_LIMIT))
        .map_err(|error| invalid(error.to_string()))?;
    Ok(json!({
        "status": status,
        "content_type": content_type,
        "headers": headers,
        "body": hex::encode(audio),
    }))
}

fn speech_reply(status: u16, headers: &Headers, body: &str) -> Result<Value, Box<HelperError>> {
    let map = header_map(headers)?;
    match decode_speech_reply(SpeechReply {
        status,
        headers: &map,
        body: body.as_bytes(),
    }) {
        Ok(response) => render_audio(response),
        Err(error) => Ok(json!({"ok": false, "status": error.status_code()})),
    }
}

fn image_params(
    model: &str,
    params: &Object,
    drop_params: bool,
) -> Result<Value, Box<HelperError>> {
    let mut optional = Object::new();
    match map_image_params(params, &mut optional, model, drop_params) {
        Ok(()) => Ok(json!({"params": optional})),
        Err(error) => Ok(json!({"ok": false, "status": error.status_code()})),
    }
}

fn image_headers(
    api_key: Option<&str>,
    environment_key: Option<&str>,
) -> Result<Value, Box<HelperError>> {
    let key = api_key.map(|text| SecretString::from(text.to_owned()));
    let environment = environment_key.map(|text| SecretString::from(text.to_owned()));
    match image_environment(&Headers::new(), key.as_ref(), environment.as_ref()) {
        Ok(headers) => Ok(serde_json::to_value(headers)?),
        Err(error) => Ok(json!({"ok": false, "status": error.status_code()})),
    }
}

fn image_reply(status: u16, body: &str, created: i64) -> Result<Value, Box<HelperError>> {
    match decode_image_generation_reply(
        ImageReply {
            status,
            body: body.as_bytes(),
        },
        created,
    ) {
        Ok(reply) => Ok(
            json!({"created": reply.created, "data": reply.data.iter().map(datum).collect::<Vec<Value>>()}),
        ),
        Err(error) => Ok(json!({"ok": false, "status": error.status_code()})),
    }
}

fn datum(image: &betterlite_openai::image_generation::reply::ImageDatum) -> Value {
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
        Step::SpeechHeaders {
            api_key,
            global_key,
            env_key,
        } => speech_headers(
            api_key.as_deref(),
            global_key.as_deref(),
            env_key.as_deref(),
        ),
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
        Step::ImageHeaders { api_key, env_key } => {
            image_headers(api_key.as_deref(), env_key.as_deref())
        }
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
