use std::future::{Future, ready};
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_models::decisions::Json;
use betterlite_openai::api::audio_file_input::ProcessedAudioFile;
use betterlite_openai::api::transcription_contract::TranscriptionResponse;
use betterlite_openai::transcription::call::{
    Pending, TranscriptionCall, TranscriptionRequest, async_audio_transcriptions,
    audio_transcriptions, finish_transcription,
};
use betterlite_openai::transcription::client::{
    ClientKind, TranscriptionConnection, select_client,
};
use betterlite_openai::transcription::config::TranscriptionFamily;
use betterlite_openai::transcription::duration::duration_number;
use betterlite_openai::transcription::error::TranscriptionError;
use betterlite_openai::transcription::logging::{
    TranscriptionLogger, TranscriptionPostCall, TranscriptionPreCall,
};
use betterlite_openai::transcription::reply::{StoredDuration, TranscriptionReply};
use betterlite_openai::transcription::request::TranscriptionData;
use betterlite_openai::transcription::transport::TranscriptionTransport;
use http::HeaderMap;
use http::header::{CONTENT_LENGTH, HeaderName, HeaderValue};
use indexmap::IndexMap;
use secrecy::SecretString;
use serde_json::{Value, json};
use uuid::Uuid;

use super::{Probe, Provider, Reply};
use crate::helpers::{HelperError, OrHelperError as _};

const FILE_NAME: &str = "a.wav";

type Started = Result<(OutboundRequest, Pending), TranscriptionError>;

struct Fixture {
    status: u16,
    headers: HeaderMap,
    body: Vec<u8>,
}

impl Fixture {
    fn new(reply: &Reply) -> Result<Self, Box<HelperError>> {
        let mut headers = HeaderMap::new();
        for (name, value) in &reply.headers {
            headers.append(
                HeaderName::from_bytes(name.as_bytes()).or_config_error()?,
                HeaderValue::from_str(value).or_config_error()?,
            );
        }
        headers.insert(CONTENT_LENGTH, HeaderValue::from(reply.body.len()));
        Ok(Self {
            status: reply.status,
            headers,
            body: reply.body.clone().into_bytes(),
        })
    }
}

impl TranscriptionTransport for Fixture {
    fn send(
        &self,
        _request: OutboundRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send {
        ready(Ok(UpstreamResponse {
            status: self.status,
            headers: self.headers.clone(),
            body: self.body.clone(),
        }))
    }
}

// the logging hook calls, already in the shape the Python side reports
#[derive(Default)]
struct Recorder {
    pre_call: Value,
    post_call: Value,
    response_headers: Value,
}

impl TranscriptionLogger for Recorder {
    fn pre_call(&mut self, record: TranscriptionPreCall<'_>) -> Result<(), PrePostError> {
        self.pre_call = json!({ "api_base": record.api_base, "data": record.data });
        Ok(())
    }

    fn response_headers(&mut self, headers: &IndexMap<String, String>) {
        self.response_headers = json!(headers);
    }

    fn post_call(&mut self, record: TranscriptionPostCall<'_>) {
        let mut original = json!(record.original_response);
        let error_text = original.is_string();
        if let Value::Object(fields) = &mut original {
            fields.retain(|_name, value| !value.is_null());
        }
        self.post_call = json!({
            "input_names_file": (!error_text).then(|| record.input.contains(FILE_NAME)),
            "data": record.data,
            "original_response": if error_text { Value::Null } else { original },
            "error_text": error_text,
        });
    }
}

fn begin(
    probe: &Probe,
    call: &TranscriptionCall<'_>,
    explicit: bool,
    recorder: &mut Recorder,
) -> Started {
    if explicit {
        let model = if probe.data_model_null {
            Json::Null
        } else {
            Json::Text(
                probe
                    .data_model
                    .clone()
                    .unwrap_or_else(|| probe.model.clone()),
            )
        };
        let data = TranscriptionData {
            model: Some(model),
            options: probe.optional_params.clone(),
        };
        return async_audio_transcriptions(call, &data, recorder);
    }
    let request = TranscriptionRequest {
        model: &probe.model,
        family: probe.provider.map(|provider| match provider {
            Provider::Whisper => TranscriptionFamily::Whisper,
            Provider::Gpt => TranscriptionFamily::Gpt,
        }),
        optional_params: probe.optional_params.clone(),
        atranscription: probe.atranscription,
    };
    audio_transcriptions(call, &request, recorder)
}

fn send(
    probe: &Probe,
    call: &TranscriptionCall<'_>,
    explicit: bool,
    recorder: &mut Recorder,
) -> Result<(Value, Result<TranscriptionReply, TranscriptionError>), Box<HelperError>> {
    let (outbound, pending) = match begin(probe, call, explicit, recorder) {
        Ok(started) => started,
        Err(error) => return Ok((Value::Null, Err(error))),
    };
    let request = request_view(&outbound);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .or_config_error()?;
    let sent = runtime.block_on(Fixture::new(&probe.reply)?.send(outbound));
    Ok((request, finish_transcription(pending, sent, recorder)))
}

fn client_view(connection: &TranscriptionConnection, kind: ClientKind) -> Value {
    select_client(connection, kind)
        .ok()
        .map_or(Value::Null, |client| {
            json!({
                "kind": if matches!(client.kind, ClientKind::Async) { "async" } else { "sync" },
                "base_url": client.base_url.as_str(),
                "max_retries": client.max_retries,
            })
        })
}

pub(super) fn exchange(probe: &Probe) -> Result<Value, Box<HelperError>> {
    let file = ProcessedAudioFile {
        file_content: b"abc".to_vec(),
        filename: FILE_NAME.to_owned(),
        content_type: String::from("audio/wav"),
    };
    let connection = TranscriptionConnection {
        api_key: probe.api_key.clone().map(SecretString::from),
        environment_api_key: None,
        api_base: probe.api_base.clone(),
        timeout: Duration::from_secs(30),
        max_retries: 2,
        shared_session: None,
    };
    let call = TranscriptionCall {
        input: FILE_NAME.to_owned(),
        file: &file,
        connection: connection.clone(),
        boundary: Uuid::from_u128(515),
    };
    let explicit = probe.data_model.is_some() || probe.data_model_null;
    let kind = if explicit || probe.atranscription {
        ClientKind::Async
    } else {
        ClientKind::Sync
    };
    let mut recorder = Recorder::default();
    let (request, outcome) = send(probe, &call, explicit, &mut recorder)?;
    let (result, error) = match outcome {
        Ok(reply) => (result_view(&reply)?, Value::Null),
        Err(failure) => (Value::Null, error_view(&failure)),
    };
    Ok(json!({
        "client": client_view(&connection, kind),
        "request": request,
        "result": result,
        "error": error,
        "pre_call": recorder.pre_call,
        "post_call": recorder.post_call,
        "response_headers": recorder.response_headers,
    }))
}

fn request_view(request: &OutboundRequest) -> Value {
    let header = |name: &str| {
        let value = request.headers.get(name)?;
        value.to_str().ok()
    };
    let mut parts = Vec::new();
    if let RequestBody::Multipart { parts: sent, .. } = &request.body {
        for part in sent {
            parts.push(json!({
                "name": part.name,
                "filename": part.file_name,
                "content_type": part.content_type,
                "body": BASE64.encode(&part.data),
            }));
        }
    }
    json!({
        "url": request.url.as_str(),
        "authorization": header("authorization"),
        "accept": header("accept"),
        "parts": parts,
    })
}

pub(super) fn response_view(response: &TranscriptionResponse) -> Value {
    let mut fields = response.extra.clone();
    let usage = fields.remove("usage");
    json!({ "text": response.text, "usage": usage, "fields": fields })
}

fn result_view(reply: &TranscriptionReply) -> Result<Value, Box<HelperError>> {
    let (recorded, found) = match reply.duration {
        StoredDuration::Absent => (false, None),
        StoredDuration::Known(found) => (true, found),
    };
    let seconds = duration_number(found).or_config_error()?;
    Ok(json!({
        "model": reply.model,
        "provider": reply.custom_llm_provider,
        "duration_recorded": recorded,
        "duration": serde_json::to_value(seconds).or_config_error()?,
        "response": response_view(&reply.response),
        "headers": reply.provider_headers.headers,
        "additional_headers": reply.provider_headers.additional_headers,
    }))
}

fn error_view(error: &TranscriptionError) -> Value {
    if let TranscriptionError::Status(status) = error {
        json!({
            "status": status.status,
            "type": status.error_type,
            "param": status.param,
            "code": status.code,
        })
    } else {
        json!({ "kind": "error" })
    }
}
