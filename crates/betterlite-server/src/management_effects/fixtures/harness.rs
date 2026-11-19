use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_auth::auth_route_patterns::canonical::RequestRouteSource;
use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_config::sections::request_body::Settings as RequestBodySettings;
use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::request_body::RequestBodyInput;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_logging::logging_metadata::exception::ExceptionFacts;
use betterlite_logging::otel::v2_callback::live_span::LiveSpan;
use betterlite_models::model_registration::JsonObject;
use opentelemetry::trace::{Tracer as _, TracerProvider as _};
use opentelemetry_sdk::trace::{InMemorySpanExporter, SdkTracerProvider};
use serde::Serialize;

use crate::failure_hook::ports::Clock;
use crate::management_effects::data::ManagementData;
use crate::management_effects::wrapper::{ManagementCall, ManagementDeps};
use crate::management_payload::otel_span::{ManagementHttpRequest, ManagementSpanDeps};

use super::cache::RecordingCache;
use super::clock::ScriptedClock;
use super::journal::{Effect, Journal};
use super::transports::{RecordingAlertTransport, RecordingSpanTransport};

pub(crate) struct Harness {
    pub(crate) events: Journal<Effect>,
    pub(crate) alert: RecordingAlertTransport,
    pub(crate) span: RecordingSpanTransport,
    pub(crate) cache: RecordingCache,
    pub(crate) clock: Arc<ScriptedClock>,
    pub(crate) body_settings: RequestBodySettings,
    pub(crate) limits: ApiLimits,
    pub(crate) default_auth: UserAPIKeyAuth,
}

impl Default for Harness {
    fn default() -> Self {
        let events = Journal::default();
        Self {
            events: events.clone(),
            alert: RecordingAlertTransport {
                events: events.clone(),
                sent: Journal::default(),
                fails: false,
            },
            span: RecordingSpanTransport {
                events: events.clone(),
                sent: Journal::default(),
                fails: false,
            },
            cache: RecordingCache {
                events,
                deletes: Journal::default(),
                failing: None,
            },
            clock: Arc::new(ScriptedClock::default()),
            body_settings: RequestBodySettings::default(),
            limits: ApiLimits::default(),
            default_auth: default_auth(),
        }
    }
}

impl Harness {
    pub(crate) fn deps(
        &self,
    ) -> ManagementDeps<'_, RecordingAlertTransport, RecordingSpanTransport, RecordingCache> {
        let clock: Arc<dyn Clock> = Arc::clone(&self.clock);
        ManagementDeps {
            alert: Some(&self.alert),
            span: ManagementSpanDeps {
                transport: Some(&self.span),
                otel_v2_enabled: false,
                body_settings: &self.body_settings,
                limits: &self.limits,
            },
            cache: &self.cache,
            default_auth: &self.default_auth,
            clock,
        }
    }
}

pub(crate) fn call<'call>(data: Option<&'call ManagementData<'call>>) -> ManagementCall<'call> {
    ManagementCall {
        function_name: "generate_key_fn",
        request_kwargs: JsonObject::new(),
        data,
        user_api_key_dict: None,
        parent_otel_span: None,
        http_request: None,
    }
}

pub(crate) fn call_with<'call>(
    data: Option<&'call ManagementData<'call>>,
    auth: &'call UserAPIKeyAuth,
    parent: Option<&'call LiveSpan>,
) -> ManagementCall<'call> {
    ManagementCall {
        function_name: "generate_key_fn",
        request_kwargs: JsonObject::new(),
        data,
        user_api_key_dict: Some(auth),
        parent_otel_span: parent,
        http_request: None,
    }
}

pub(crate) fn http_request<'path>(path: &'path str) -> ManagementHttpRequest<'path> {
    ManagementHttpRequest {
        route: RequestRouteSource {
            scope: Ok(None),
            url_path: path,
        },
        body: RequestBodyInput {
            method: "POST".to_owned(),
            route_path: path.to_owned(),
            headers: BTreeMap::from([("content-type".to_owned(), "application/json".to_owned())]),
            query_params: BTreeMap::new(),
            body: br#"{"key": "sk-created"}"#.to_vec(),
            client_requested_model: None,
            cached_body: None,
            form_fields: None,
        },
    }
}

pub(crate) fn default_auth() -> UserAPIKeyAuth {
    UserAPIKeyAuth::construct(AuthInput::Dict(BTreeMap::new()))
        .expect("the default auth fixture constructs")
}

pub(crate) fn actor_auth() -> UserAPIKeyAuth {
    let fields: BTreeMap<String, JsonData> = serde_json::from_str(
        r#"{"user_id": "u1", "user_role": "proxy_admin", "key_alias": "ops"}"#,
    )
    .expect("the actor fixture parses");
    UserAPIKeyAuth::construct(AuthInput::Dict(fields)).expect("the actor fixture constructs")
}

pub(crate) fn parent_span() -> LiveSpan {
    let provider = SdkTracerProvider::builder()
        .with_simple_exporter(InMemorySpanExporter::default())
        .build();
    let tracer = provider.tracer("management-effects");
    LiveSpan::from(tracer.start("parent"))
}

pub(crate) fn exception() -> ExceptionFacts {
    serde_json::from_str(r#"{"class_name": "ProxyException", "message": "boom"}"#)
        .expect("the fixture exception parses")
}

#[derive(Debug, PartialEq, Eq, Serialize)]
pub(crate) struct CreatedKey {
    pub(crate) alias: String,
    pub(crate) user_id: String,
}

pub(crate) fn created_key() -> CreatedKey {
    CreatedKey {
        alias: "ops".to_owned(),
        user_id: "u1".to_owned(),
    }
}
