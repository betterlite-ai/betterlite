use std::collections::VecDeque;
use std::future::{Future, ready};
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_core::relay::assemblyai::{
    AssemblyCredentialSource, AssemblyTranscriptLog, AssemblyTranscriptLogSink,
    AssemblyTranscriptTransport,
};
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_models::decisions::Json;
use http::header::HeaderMap;
use indexmap::IndexMap;
use secrecy::SecretString;
use serde::Deserialize;
use url::Url;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Reply {
    status: u16,
    body: Json,
}

pub(super) struct Recorded {
    pub(super) url: Url,
    pub(super) headers: HeaderMap,
}

#[derive(Clone)]
pub(super) struct ScriptedTransport {
    replies: Arc<Mutex<VecDeque<Reply>>>,
    repeat: Reply,
    pub(super) requests: Arc<Mutex<Vec<Recorded>>>,
}

impl ScriptedTransport {
    pub(super) fn new(replies: &[Reply]) -> Self {
        let repeat = replies.last().cloned().unwrap_or(Reply {
            status: 200,
            body: Json::Object(IndexMap::new()),
        });
        Self {
            replies: Arc::new(Mutex::new(VecDeque::from(replies.to_vec()))),
            repeat,
            requests: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

impl AssemblyTranscriptTransport for ScriptedTransport {
    fn get(
        &self,
        url: Url,
        headers: HeaderMap,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Recorded { url, headers });
        let reply = self
            .replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .unwrap_or_else(|| self.repeat.clone());
        ready(
            serde_json::to_vec(&reply.body)
                .map_err(|_error| TransportError::Closed)
                .map(|body| UpstreamResponse {
                    status: reply.status,
                    headers: HeaderMap::new(),
                    body,
                }),
        )
    }
}

pub(super) struct StaticCredentials {
    pub(super) key: Option<SecretString>,
}

impl AssemblyCredentialSource for StaticCredentials {
    fn get_credentials(
        &self,
        _custom_llm_provider: &str,
        _region_name: Option<&str>,
    ) -> impl Future<Output = Option<SecretString>> + Send {
        ready(self.key.clone())
    }
}

#[derive(Clone, Default)]
pub(super) struct RecordingSink {
    pub(super) logs: Arc<Mutex<Vec<AssemblyTranscriptLog>>>,
}

impl AssemblyTranscriptLogSink for RecordingSink {
    async fn handle_logging(&self, call: AssemblyTranscriptLog) {
        self.logs
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(call);
    }
}
