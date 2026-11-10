use std::collections::BTreeMap;
use std::io;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::tls::Verification;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::{ProxiedClient, base_builder};
use betterlite_http::user_url::fetch::UserUrlFetcher;
use betterlite_http::user_url::validate::{SystemResolver, UrlValidator};
use tracing::subscriber::{DefaultGuard, set_default};

use crate::arguments::json::{Json, Object};
use crate::bedrock_images::remote::{RemoteImage, TRANSPORT_LIMITS};
use crate::bedrock_processor::conversation::{Conversion, converse_messages};
use crate::bedrock_processor::error::ProcessorError;
use crate::bedrock_tool_results::invoke::CachePoints;

pub(super) const DEFAULT_POINT: &str = r#"{"type":"default"}"#;

pub(super) fn json(raw: &str) -> Json {
    serde_json::from_str(raw).unwrap()
}

pub(super) fn list(raw: &str) -> Vec<Json> {
    serde_json::from_str(raw).unwrap()
}

pub(super) fn object(raw: &str) -> Object {
    serde_json::from_str(raw).unwrap()
}

#[derive(Default)]
pub(super) struct Settings<'call> {
    pub(super) model: Option<&'call str>,
    pub(super) user_continue_message: Option<&'call Json>,
    pub(super) assistant_continue_message: Option<&'call Json>,
    pub(super) modify_params: bool,
}

impl<'call> Settings<'call> {
    #[must_use]
    pub(super) fn with_modify_params(mut self) -> Self {
        self.modify_params = true;
        self
    }

    #[must_use]
    pub(super) fn with_model(mut self, model: &'call str) -> Self {
        self.model = Some(model);
        self
    }

    #[must_use]
    pub(super) fn with_user_continue(mut self, message: &'call Json) -> Self {
        self.user_continue_message = Some(message);
        self
    }

    #[must_use]
    pub(super) fn with_assistant_continue(mut self, message: &'call Json) -> Self {
        self.assistant_continue_message = Some(message);
        self
    }
}

pub(super) struct RecordingCachePoints {
    point: Option<Object>,
    seen: Mutex<Vec<(Json, Option<String>)>>,
}

impl RecordingCachePoints {
    pub(super) fn accepting() -> Self {
        Self {
            point: Some(object(DEFAULT_POINT)),
            seen: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn rejecting() -> Self {
        Self {
            point: None,
            seen: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn with_point(point: &str) -> Self {
        Self {
            point: Some(object(point)),
            seen: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn seen(&self) -> Vec<(Json, Option<String>)> {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl CachePoints for RecordingCachePoints {
    fn cache_point_block(&self, cache_control: &Json, model: Option<&str>) -> Option<Object> {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((cache_control.clone(), model.map(str::to_owned)));
        self.point.clone()
    }
}

pub(super) struct Harness {
    pub(super) remote: RemoteImage<SystemResolver>,
    pub(super) cache: RecordingCachePoints,
}

impl Harness {
    pub(super) fn new(cache: RecordingCachePoints) -> Self {
        let policy =
            Arc::new(ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).unwrap());
        let client = ProxiedClient::build(base_builder(), &policy, TRANSPORT_LIMITS).unwrap();
        let validator = UrlValidator::new(SystemResolver, &[], &Verification::Enabled);
        Self {
            remote: RemoteImage {
                client,
                urls: UserUrlFetcher {
                    validator,
                    validation: true,
                },
                deadline: TRANSPORT_LIMITS.read_timeout,
                max_download_size_mb: 50.0,
            },
            cache,
        }
    }

    pub(super) async fn convert(
        &self,
        messages: &[Json],
        settings: &Settings<'_>,
    ) -> Result<Vec<Json>, ProcessorError> {
        let conversion = Conversion {
            model: settings.model,
            cache_points: &self.cache,
            remote: &self.remote,
            user_continue_message: settings.user_continue_message,
            assistant_continue_message: settings.assistant_continue_message,
            modify_params: settings.modify_params,
        };
        let new_id = || "call_generated".to_owned();
        converse_messages(messages, &conversion, &new_id).await
    }
}

#[derive(Clone, Default)]
pub(super) struct Captured(Arc<Mutex<Vec<u8>>>);

impl Captured {
    pub(super) fn text(&self) -> String {
        String::from_utf8(
            self.0
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone(),
        )
        .unwrap()
    }
}

impl io::Write for Captured {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

pub(super) fn capture() -> (Captured, DefaultGuard) {
    let captured = Captured::default();
    let writer = captured.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(move || writer.clone())
        .with_max_level(tracing::Level::DEBUG)
        .with_ansi(false)
        .finish();
    (captured, set_default(subscriber))
}
