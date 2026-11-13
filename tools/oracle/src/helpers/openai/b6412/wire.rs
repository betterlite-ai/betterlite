use std::collections::BTreeMap;

use betterlite_openai::chat::ChatMessage;
use betterlite_openai::chat_content::RefuseDownloads;
use betterlite_openai::ovhcloud_audio::reply::decode_transcription_reply as ovhcloud_reply;
use betterlite_openai::ovhcloud_audio::request::{
    complete_url as ovhcloud_audio_url, transcription_request as ovhcloud_form,
    validate_environment as ovhcloud_audio_headers,
};
use betterlite_openai::ovhcloud_chat::chunk::chunk_parser;
use betterlite_openai::ovhcloud_chat::request::{
    ChatTransform, complete_url as chat_url, transform_request,
};
use betterlite_openai::ovhcloud_embedding::reply::decode_embedding_reply;
use betterlite_openai::ovhcloud_embedding::request::validate_environment as embedding_headers;
use betterlite_openai::scaleway_audio::reply::decode_transcription_reply as scaleway_reply;
use betterlite_openai::scaleway_audio::request::{
    complete_url as scaleway_url, transcription_request as scaleway_form,
    validate_environment as scaleway_headers,
};
use betterlite_prompts::arguments::json::Object as JsonObject;
use secrecy::SecretString;
use serde_json::{Value, json};

use super::{
    AudioProvider, ChatRequestCall, EmbeddingCall, HeaderCall, HeaderProvider, ReplyCall,
    TranscriptionCall, env_key, fields_pairs, file_value, processed,
};

pub(super) fn chat_request_value(call: &ChatRequestCall) -> Value {
    let messages: &[ChatMessage] = &[];
    let environment = BTreeMap::<String, String>::new();
    let transform = ChatTransform {
        model: &call.model,
        messages,
        optional_params: &call.optional_params,
        environment: &environment,
        downloads: &RefuseDownloads,
    };
    let url = chat_url(call.api_base.as_deref());
    match transform_request(&transform) {
        Ok(body) => json!({"url": url, "ok": true, "value": body}),
        Err(_) => json!({"url": url, "ok": false}),
    }
}

pub(super) fn chunk_value(chunk: &JsonObject) -> Value {
    match chunk_parser(chunk) {
        Ok(parsed) => json!({"ok": true, "value": parsed}),
        Err(error) => json!({"raised": error.status_code}),
    }
}

pub(super) fn header_value(provider: HeaderProvider, call: &HeaderCall) -> Value {
    let explicit = call.api_key.clone().map(SecretString::from);
    let from_env = env_key(&call.env, provider.env_name());
    if provider == HeaderProvider::ScalewayAudio {
        return match scaleway_headers(&call.headers, explicit.as_ref(), from_env.as_ref()) {
            Ok(headers) => json!({"ok": true, "headers": headers}),
            Err(error) => json!({"raised": error.status_code}),
        };
    }
    let headers = if provider == HeaderProvider::OvhcloudEmbedding {
        embedding_headers(&call.headers, explicit.as_ref(), from_env.as_ref())
    } else {
        ovhcloud_audio_headers(&call.headers, explicit.as_ref(), from_env.as_ref())
    };
    json!({"ok": true, "headers": headers})
}

pub(super) fn embedding_reply_value(call: &ReplyCall) -> Value {
    match decode_embedding_reply(call.body.as_bytes(), call.status, &call.headers) {
        Ok(reply) => json!({"ok": true, "value": reply}),
        Err(error) => json!({"raised": error.status_code}),
    }
}

pub(super) fn transcription_request_value(
    provider: AudioProvider,
    call: &TranscriptionCall,
) -> Value {
    let file = match processed(call) {
        Ok(file) => file,
        Err(failure) => return failure,
    };
    let (url, keyed, request) = match provider {
        AudioProvider::Ovhcloud => (
            ovhcloud_audio_url(call.api_base.as_deref()),
            HeaderProvider::OvhcloudAudio,
            ovhcloud_form(&call.model, &file, &call.optional_params),
        ),
        AudioProvider::Scaleway => (
            scaleway_url(call.api_base.as_deref()),
            HeaderProvider::ScalewayAudio,
            scaleway_form(&call.model, &file, &call.optional_params),
        ),
    };
    let key_call = HeaderCall {
        api_key: None,
        headers: BTreeMap::new(),
        env: call.env.clone(),
    };
    json!({
        "url": url,
        "headers": header_value(keyed, &key_call),
        "request": {
            "form": fields_pairs(&request.fields),
            "file": file_value(&request.file),
        },
    })
}

pub(super) fn transcription_reply_value(provider: AudioProvider, call: &ReplyCall) -> Value {
    let reply = match provider {
        AudioProvider::Ovhcloud => ovhcloud_reply(call.body.as_bytes(), call.status, &call.headers)
            .map(|transcription| json!({"text": transcription.response.text})),
        AudioProvider::Scaleway => {
            scaleway_reply(call.body.as_bytes(), call.status, &call.headers).map(scaleway_text)
        }
    };
    match reply {
        Ok(value) => json!({"ok": true, "value": value}),
        Err(error) => json!({"raised": error.status_code}),
    }
}

fn scaleway_text(transcription: betterlite_openai::scaleway_audio::reply::Transcription) -> Value {
    let mut value = json!({"text": transcription.response.text});
    for name in ["segments", "language"] {
        if let Some(field) = transcription.response.extra.get(name) {
            value[name] = json!(field);
        }
    }
    value
}
