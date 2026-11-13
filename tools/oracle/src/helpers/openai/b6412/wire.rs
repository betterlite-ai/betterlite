use betterlite_openai::api::audio_file_input::ProcessedAudioFile;
use betterlite_openai::chat::ChatMessage;
use betterlite_openai::chat_content::RefuseDownloads;
use betterlite_openai::openai_base_config::Headers;
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
use betterlite_openai::ovhcloud_embedding::request::transform_embedding_request;
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
    TranscriptionCall, env_key, fields_pairs, file_value, pairs, processed,
};

pub(super) fn chat_request_value(call: &ChatRequestCall) -> Value {
    let messages: &[ChatMessage] = &[];
    let environment = std::collections::BTreeMap::<String, String>::new();
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
        Err(error) => json!({"ok": false, "status": error.status_code}),
    }
}

pub(super) fn header_value(provider: HeaderProvider, call: &HeaderCall) -> Value {
    let explicit = call.api_key.clone().map(SecretString::from);
    let from_env = env_key(&call.env, provider.env_name());
    if provider == HeaderProvider::ScalewayAudio {
        return match scaleway_headers(&call.headers, explicit.as_ref(), from_env.as_ref()) {
            Ok(headers) => json!({"ok": true, "headers": headers}),
            Err(error) => json!({"ok": false, "status": error.status_code}),
        };
    }
    let headers = if provider == HeaderProvider::OvhcloudEmbedding {
        embedding_headers(&call.headers, explicit.as_ref(), from_env.as_ref())
    } else {
        ovhcloud_audio_headers(&call.headers, explicit.as_ref(), from_env.as_ref())
    };
    json!({"ok": true, "headers": headers})
}

pub(super) fn embedding_request_value(call: &EmbeddingCall) -> Value {
    let body = transform_embedding_request(&call.model, &call.input, &JsonObject::new());
    json!({"value": pairs(&body)})
}

pub(super) fn embedding_reply_value(call: &ReplyCall) -> Value {
    match decode_embedding_reply(call.body.as_bytes(), call.status, &call.headers) {
        Ok(reply) => json!({"ok": true, "value": reply}),
        Err(error) => json!({"ok": false, "status": error.status_code}),
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
    match provider {
        AudioProvider::Ovhcloud => ovhcloud_transcription_value(call, &file),
        AudioProvider::Scaleway => scaleway_transcription_value(call, &file),
    }
}

fn ovhcloud_transcription_value(call: &TranscriptionCall, file: &ProcessedAudioFile) -> Value {
    let from_env = env_key(&call.env, "OVHCLOUD_API_KEY");
    let headers = ovhcloud_audio_headers(&Headers::new(), None, from_env.as_ref());
    let request = ovhcloud_form(&call.model, file, &call.optional_params);
    json!({
        "url": ovhcloud_audio_url(call.api_base.as_deref()),
        "ok": true,
        "headers": headers,
        "form": fields_pairs(&request.fields),
        "file": file_value(&request.file),
    })
}

fn scaleway_transcription_value(call: &TranscriptionCall, file: &ProcessedAudioFile) -> Value {
    let url = scaleway_url(call.api_base.as_deref());
    let from_env = env_key(&call.env, "SCW_SECRET_KEY");
    let Ok(headers) = scaleway_headers(&Headers::new(), None, from_env.as_ref()) else {
        return json!({"url": url, "ok": false, "status": 401});
    };
    let request = scaleway_form(&call.model, file, &call.optional_params);
    json!({
        "url": url,
        "ok": true,
        "headers": headers,
        "form": fields_pairs(&request.fields),
        "file": file_value(&request.file),
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
        Err(error) => json!({"ok": false, "status": error.status_code}),
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
