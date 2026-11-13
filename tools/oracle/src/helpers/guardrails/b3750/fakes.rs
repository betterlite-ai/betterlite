use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::aim::error::AimError;
use betterlite_guardrails::aim::settings::{AimGuardrail, AimParams};
use betterlite_guardrails::aim::stream::{
    AimFrameReader, AimFrameWriter, AimSocket, AimSocketConnector, AimSocketTimeouts,
};
use betterlite_guardrails::aim::transport::{AimHttpResponse, AimPoster};
use reqwest::StatusCode;
use reqwest::header::HeaderMap;
use secrecy::SecretString;

use super::{Settings, failure};

pub(super) struct RecordedPost {
    pub(super) url: String,
    pub(super) headers: HeaderMap,
    pub(super) body: Vec<u8>,
}

pub(super) struct RecordingPoster {
    pub(super) status: StatusCode,
    pub(super) body: Vec<u8>,
    pub(super) posts: Mutex<Vec<RecordedPost>>,
}

#[async_trait]
impl AimPoster for RecordingPoster {
    async fn post(
        &self,
        url: &str,
        headers: &HeaderMap,
        body: &[u8],
        _timeout: Option<Duration>,
    ) -> Result<AimHttpResponse, AimError> {
        let mut posts = self.posts.lock().unwrap_or_else(PoisonError::into_inner);
        posts.push(RecordedPost {
            url: url.to_owned(),
            headers: headers.clone(),
            body: body.to_vec(),
        });
        Ok(AimHttpResponse {
            status: self.status,
            body: self.body.clone(),
        })
    }
}

#[derive(Default)]
struct ScriptedFrames {
    incoming: Mutex<VecDeque<Option<String>>>,
}

struct FakeReader {
    frames: Arc<ScriptedFrames>,
}

#[async_trait]
impl AimFrameReader for FakeReader {
    async fn recv_text(&mut self) -> Result<Option<String>, AimError> {
        let mut incoming = self
            .frames
            .incoming
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        Ok(incoming.pop_front().flatten())
    }
}

struct FakeWriter;

#[async_trait]
impl AimFrameWriter for FakeWriter {
    async fn send_text(&mut self, _text: String) -> Result<(), AimError> {
        Ok(())
    }
}

struct FakeConnector {
    frames: Arc<ScriptedFrames>,
}

#[async_trait]
impl AimSocketConnector for FakeConnector {
    async fn connect(
        &self,
        _url: &str,
        _headers: &HeaderMap,
        _timeouts: AimSocketTimeouts,
    ) -> Result<AimSocket, AimError> {
        Ok(AimSocket {
            reader: Box::new(FakeReader {
                frames: Arc::clone(&self.frames),
            }),
            writer: Box::new(FakeWriter),
        })
    }
}

pub(super) struct Built {
    pub(super) guardrail: AimGuardrail,
    pub(super) poster: Arc<RecordingPoster>,
}

pub(super) fn build(
    settings: &Settings,
    environment: &BTreeMap<String, String>,
    answer: &GuardrailValue,
    status: u16,
    frames: &[GuardrailValue],
) -> Result<Built, Box<crate::helpers::HelperError>> {
    let body =
        serde_json::to_vec(answer).map_err(|error| failure("ValueError", error.to_string()))?;
    let poster = Arc::new(RecordingPoster {
        status: StatusCode::from_u16(status)
            .map_err(|error| failure("ValueError", error.to_string()))?,
        body,
        posts: Mutex::new(Vec::new()),
    });
    let scripted = Arc::new(ScriptedFrames::default());
    {
        let mut incoming = scripted
            .incoming
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        for frame in frames {
            let text = serde_json::to_string(frame)
                .map_err(|error| failure("ValueError", error.to_string()))?;
            incoming.push_back(Some(text));
        }
    }
    let connector = Arc::new(FakeConnector {
        frames: Arc::clone(&scripted),
    });
    let params = AimParams {
        guardrail_name: settings.guardrail_name.clone(),
        api_key: settings.api_key.clone().map(SecretString::from),
        api_base: settings.api_base.clone(),
        inspect_embeddings: settings.inspect_embeddings,
        litellm_version: settings.litellm_version.clone(),
    };
    let poster_port: Arc<dyn AimPoster> = Arc::clone(&poster);
    let connector_port: Arc<dyn AimSocketConnector> = Arc::clone(&connector);
    let guardrail = AimGuardrail::new(
        params,
        environment,
        poster_port,
        connector_port,
        Some(Duration::from_secs(30)),
    )
    .map_err(|error| match error {
        AimError::MissingSecrets => failure("AimMissingSecrets", error.to_string()),
        _ => failure("AimError", error.to_string()),
    })?;
    Ok(Built { guardrail, poster })
}
