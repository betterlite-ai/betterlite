use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::aim::error::AimError;
use betterlite_guardrails::aim::settings::{AimGuardrail, AimParams};
use betterlite_guardrails::aim::stream::{
    AimFrameReader, AimFrameWriter, AimSocket, AimSocketConnector,
};
use betterlite_guardrails::aim::transport::{AimHttpResponse, AimPoster};
use reqwest::StatusCode;
use reqwest::header::HeaderMap;
use secrecy::SecretString;

use super::{Settings, failure};

pub(super) struct Recorded {
    pub(super) url: String,
    pub(super) headers: HeaderMap,
    pub(super) body: Vec<u8>,
}

#[derive(Default)]
pub(super) struct State {
    pub(super) posts: Mutex<Vec<Recorded>>,
    pub(super) incoming: Mutex<VecDeque<Option<String>>>,
}

pub(super) struct Fake {
    pub(super) state: Arc<State>,
    pub(super) status: StatusCode,
    pub(super) body: Vec<u8>,
}

#[async_trait]
impl AimPoster for Fake {
    async fn post(
        &self,
        url: &str,
        headers: &HeaderMap,
        body: &[u8],
        _timeout: Option<Duration>,
    ) -> Result<AimHttpResponse, AimError> {
        let mut posts = self
            .state
            .posts
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        posts.push(Recorded {
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

#[async_trait]
impl AimFrameReader for Fake {
    async fn recv_text(&mut self) -> Result<Option<String>, AimError> {
        let mut incoming = self
            .state
            .incoming
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        Ok(incoming.pop_front().flatten())
    }
}

#[async_trait]
impl AimFrameWriter for Fake {
    async fn send_text(&mut self, _text: String) -> Result<(), AimError> {
        Ok(())
    }
}

#[async_trait]
impl AimSocketConnector for Fake {
    async fn connect(
        &self,
        _url: &str,
        _headers: &HeaderMap,
        _open: Duration,
        _read: Duration,
    ) -> Result<AimSocket, AimError> {
        Ok(AimSocket {
            reader: Box::new(Fake {
                state: Arc::clone(&self.state),
                status: self.status,
                body: Vec::new(),
            }),
            writer: Box::new(Fake {
                state: Arc::clone(&self.state),
                status: self.status,
                body: Vec::new(),
            }),
        })
    }
}

pub(super) struct Built {
    pub(super) guardrail: AimGuardrail,
    pub(super) state: Arc<State>,
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
    let state = Arc::new(State::default());
    {
        let mut incoming = state
            .incoming
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        for frame in frames {
            let text = serde_json::to_string(frame)
                .map_err(|error| failure("ValueError", error.to_string()))?;
            incoming.push_back(Some(text));
        }
    }
    let fake = Arc::new(Fake {
        state: Arc::clone(&state),
        status: StatusCode::from_u16(status)
            .map_err(|error| failure("ValueError", error.to_string()))?,
        body,
    });
    let params = AimParams {
        guardrail_name: settings.guardrail_name.clone(),
        api_key: settings.api_key.clone().map(SecretString::from),
        api_base: settings.api_base.clone(),
        inspect_embeddings: settings.inspect_embeddings,
        litellm_version: settings.litellm_version.clone(),
    };
    let poster: Arc<dyn AimPoster> = Arc::clone(&fake);
    let connector: Arc<dyn AimSocketConnector> = fake;
    let guardrail = AimGuardrail::new(
        params,
        environment,
        poster,
        connector,
        Some(Duration::from_secs(30)),
    )
    .map_err(|error| match error {
        AimError::MissingSecrets => failure("AimMissingSecrets", error.to_string()),
        _ => failure("AimError", error.to_string()),
    })?;
    Ok(Built { guardrail, state })
}
