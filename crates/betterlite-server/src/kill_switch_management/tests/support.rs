#![cfg(test)]

use std::collections::{BTreeMap, VecDeque};
use std::future::{Future, ready};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use betterlite_core::agents::kill_switch::ApiKeyHeader;
use betterlite_core::agents::{AgentKillSwitchAuth, AgentKillSwitchConfig, AgentKillSwitchMethod};
use chrono::{DateTime, FixedOffset};
use secrecy::SecretString;
use thiserror::Error;

use super::client::{KillSwitchHttpClient, KillSwitchResponse, KillSwitchSendOptions};
use super::request::KillSwitchRequest;
use crate::audit_events::sink::AuditClock;

pub(crate) fn secret(value: &str) -> SecretString {
    SecretString::from(value)
}

pub(crate) fn bearer_auth(token: &str) -> AgentKillSwitchAuth {
    AgentKillSwitchAuth::Bearer {
        token: secret(token),
    }
}

pub(crate) fn api_key_auth(header_name: &str, api_key: &str) -> AgentKillSwitchAuth {
    AgentKillSwitchAuth::ApiKey {
        header_name: ApiKeyHeader(String::from(header_name)),
        api_key: secret(api_key),
    }
}

pub(crate) fn default_api_key_auth(api_key: &str) -> AgentKillSwitchAuth {
    AgentKillSwitchAuth::ApiKey {
        header_name: ApiKeyHeader::default(),
        api_key: secret(api_key),
    }
}

pub(crate) fn basic_auth(username: &str, password: &str) -> AgentKillSwitchAuth {
    AgentKillSwitchAuth::Basic {
        username: String::from(username),
        password: secret(password),
    }
}

pub(crate) fn config_with_auth(auth: Option<AgentKillSwitchAuth>) -> AgentKillSwitchConfig {
    AgentKillSwitchConfig {
        url: String::from("https://agent.example/stop"),
        method: AgentKillSwitchMethod::Post,
        headers: BTreeMap::new(),
        query_params: BTreeMap::new(),
        body: None,
        auth,
    }
}

#[derive(Debug, Error)]
#[error("{message}")]
pub(crate) struct FakeError {
    pub(crate) message: String,
}

#[derive(Clone, Default)]
pub(crate) struct ResponseProbe {
    pub(crate) chunks_read: usize,
    pub(crate) closed: bool,
}

pub(crate) struct BuiltRequest {
    pub(crate) request: KillSwitchRequest,
    pub(crate) timeout: Duration,
}

#[derive(Clone)]
pub(crate) struct Sent {
    pub(crate) request: KillSwitchRequest,
    pub(crate) timeout: Duration,
    pub(crate) options: KillSwitchSendOptions,
}

pub(crate) struct FakeClient {
    pub(crate) status: u16,
    pub(crate) chunks: Vec<Result<String, String>>,
    pub(crate) build_error: Option<String>,
    pub(crate) send_error: Option<String>,
    pub(crate) close_error: Option<String>,
    pub(crate) probe: Arc<Mutex<ResponseProbe>>,
    pub(crate) sent: Mutex<Vec<Sent>>,
}

pub(crate) struct FakeResponse {
    status: u16,
    chunks: VecDeque<Result<String, String>>,
    close_error: Option<String>,
    probe: Arc<Mutex<ResponseProbe>>,
}

pub(crate) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl FakeClient {
    pub(crate) fn scripted(status: u16, chunks: Vec<Result<String, String>>) -> Self {
        Self {
            status,
            chunks,
            build_error: None,
            send_error: None,
            close_error: None,
            probe: Arc::new(Mutex::new(ResponseProbe::default())),
            sent: Mutex::new(Vec::new()),
        }
    }
}

pub(crate) fn probe(client: &FakeClient) -> ResponseProbe {
    lock(&client.probe).clone()
}

pub(crate) fn first_sent(client: &FakeClient) -> Option<Sent> {
    lock(&client.sent).first().cloned()
}

impl KillSwitchHttpClient for FakeClient {
    type Error = FakeError;
    type Request = BuiltRequest;
    type Response = FakeResponse;

    fn build_request(
        &self,
        request: &KillSwitchRequest,
        timeout: Duration,
    ) -> Result<BuiltRequest, FakeError> {
        self.build_error.as_ref().map_or_else(
            || {
                Ok(BuiltRequest {
                    request: request.clone(),
                    timeout,
                })
            },
            |message| {
                Err(FakeError {
                    message: message.clone(),
                })
            },
        )
    }

    fn send(
        &self,
        request: BuiltRequest,
        options: KillSwitchSendOptions,
    ) -> impl Future<Output = Result<FakeResponse, FakeError>> + Send {
        lock(&self.sent).push(Sent {
            request: request.request,
            timeout: request.timeout,
            options,
        });
        ready(self.send_error.as_ref().map_or_else(
            || {
                Ok(FakeResponse {
                    status: self.status,
                    chunks: self.chunks.iter().cloned().collect(),
                    close_error: self.close_error.clone(),
                    probe: Arc::clone(&self.probe),
                })
            },
            |message| {
                Err(FakeError {
                    message: message.clone(),
                })
            },
        ))
    }
}

impl KillSwitchResponse for FakeResponse {
    type Error = FakeError;

    fn status(&self) -> u16 {
        self.status
    }

    fn next_text_chunk(
        &mut self,
    ) -> impl Future<Output = Option<Result<String, FakeError>>> + Send {
        let chunk = self.chunks.pop_front();
        let mut probe = lock(&self.probe);
        let read = probe.chunks_read.saturating_add(1);
        probe.chunks_read = read;
        drop(probe);
        ready(chunk.map(|result| result.map_err(|message| FakeError { message })))
    }

    fn close(&mut self) -> impl Future<Output = Result<(), FakeError>> + Send {
        let mut probe = lock(&self.probe);
        probe.closed = true;
        drop(probe);
        ready(
            self.close_error
                .take()
                .map_or(Ok(()), |message| Err(FakeError { message })),
        )
    }
}

pub(crate) struct FixedClock {
    pub(crate) now: DateTime<FixedOffset>,
}

impl AuditClock for FixedClock {
    fn now(&self) -> DateTime<FixedOffset> {
        self.now
    }
}
