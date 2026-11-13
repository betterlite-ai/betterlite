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
    transcription_value(provider, call, &file)
}

// both transcription requests answer with the url, the headers, the ordered form and the file tuple
fn form_answer(
    url: String,
    headers: Headers,
    fields: &[(String, Value)],
    file: &ProcessedAudioFile,
) -> Value {
    json!({
        "url": url,
        "ok": true,
        "headers": headers,
        "form": fields_pairs(fields),
        "file": file_value(file),
    })
}

fn transcription_value(
    provider: AudioProvider,
    call: &TranscriptionCall,
    file: &ProcessedAudioFile,
) -> Value {
    let url = match provider {
        AudioProvider::Ovhcloud => ovhcloud_audio_url(call.api_base.as_deref()),
        AudioProvider::Scaleway => scaleway_url(call.api_base.as_deref()),
    };
    let headers = match provider {
        AudioProvider::Ovhcloud => {
            let from_env = env_key(&call.env, "OVHCLOUD_API_KEY");
            ovhcloud_audio_headers(&Headers::new(), None, from_env.as_ref())
        }
        AudioProvider::Scaleway => {
            let from_env = env_key(&call.env, "SCW_SECRET_KEY");
            match scaleway_headers(&Headers::new(), None, from_env.as_ref()) {
                Ok(headers) => headers,
                Err(error) => {
                    return json!({"url": url, "ok": false, "status": error.status_code});
                }
            }
        }
    };
    let request = match provider {
        AudioProvider::Ovhcloud => ovhcloud_form(&call.model, file, &call.optional_params),
        AudioProvider::Scaleway => scaleway_form(&call.model, file, &call.optional_params),
    };
    form_answer(url, headers, &request.fields, &request.file)
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
