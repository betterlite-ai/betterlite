use std::collections::BTreeMap;
use std::convert::Infallible;
use std::future::Future;
use std::sync::Arc;

use async_trait::async_trait;
use axum::Extension;
use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_config::sections::alert_settings::types::AlertType;
use betterlite_config::sections::request_body::Settings as RequestBodySettings;
use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_logging::otel::service::management::ManagementPayload;
use betterlite_logging::otel::v2_callback::live_span::LiveSpan;
use betterlite_mcp::registry::resolution::ServerAccessCheck;
use betterlite_models::model_registration::JsonObject;
use chrono::{DateTime, FixedOffset, Utc};
use serde::Serialize;

use crate::alerting::model_alerts::key_event::VirtualKeyEvent;
use crate::auth::auth_identity::AuthContext;
use crate::failure_hook::ports::Clock;
use crate::management_effects::eviction::ManagementCacheEviction;
use crate::management_effects::wrapper::{
    ManagementCall, ManagementDeps, ManagementFailure, management_endpoint_wrapper,
};
use crate::management_payload::alerts::ManagementAlertTransport;
use crate::management_payload::otel_span::{ManagementSpanDeps, ManagementSpanTransport};
use crate::mcp::management_server_access::AccessibleServerSources;

use super::UserEnvRoutesState;
use super::error::{RouteError, UserEnvRouteError};

impl<Access, Sources> UserEnvRoutesState<Access, Sources>
where
    Access: ServerAccessCheck,
    Sources: AccessibleServerSources,
{
    // B865's wrapper around one handler: the routes send no alert and no span, and evict no cached key
    pub(crate) async fn manage<R, Body, Fut>(
        &self,
        caller: Option<Extension<AuthContext>>,
        function_name: &'static str,
        body: Body,
    ) -> Result<R, RouteError<Access, Sources>>
    where
        R: Serialize + Send + Sync,
        Body: FnOnce(UserAPIKeyAuth) -> Fut + Send,
        Fut: Future<Output = Result<R, RouteError<Access, Sources>>> + Send,
    {
        let source_key = caller
            .as_ref()
            .and_then(|extension| extension.0.principal.source_key.0.clone());
        let auth = match &source_key {
            Some(key) => key.clone(),
            None => UserAPIKeyAuth::construct(AuthInput::Dict(BTreeMap::new()))
                .map_err(UserEnvRouteError::DefaultAuth)?,
        };
        let limits = ApiLimits::default();
        let body_settings = RequestBodySettings::default();
        let effects = NoEffects;
        let deps = management_deps(&auth, &limits, &body_settings, &effects);
        let call = ManagementCall {
            function_name,
            request_kwargs: JsonObject::new(),
            data: None,
            user_api_key_dict: source_key.as_ref(),
            parent_otel_span: None,
            http_request: None,
        };
        let body_auth = auth.clone();
        management_endpoint_wrapper::<
            R,
            RouteError<Access, Sources>,
            _,
            NoEffects,
            NoEffects,
            NoEffects,
        >(call, deps, async move {
            body(body_auth).await.map_err(|error| {
                let exception = error.facts();
                ManagementFailure { exception, error }
            })
        })
        .await
    }
}

// The wrapper sees no alert transport, no span transport and no key cache: these routes have none
fn management_deps<'deps>(
    auth: &'deps UserAPIKeyAuth,
    limits: &'deps ApiLimits,
    body_settings: &'deps RequestBodySettings,
    effects: &'deps NoEffects,
) -> ManagementDeps<'deps, NoEffects, NoEffects, NoEffects> {
    ManagementDeps {
        alert: None,
        span: ManagementSpanDeps {
            transport: None,
            otel_v2_enabled: false,
            body_settings,
            limits,
        },
        cache: effects,
        default_auth: auth,
        clock: Arc::new(SystemClock),
    }
}

struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> DateTime<FixedOffset> {
        Utc::now().fixed_offset()
    }
}

// The wrapper's three effects are wired, and a per-user env var call has no alert name, parent span or key id
struct NoEffects;

#[async_trait]
impl ManagementAlertTransport for NoEffects {
    type Error = Infallible;

    async fn send_management_event_alert(
        &self,
        _key_event: &VirtualKeyEvent,
        _alert_type: AlertType,
        _event_name: &str,
    ) -> Result<(), Infallible> {
        Ok(())
    }
}

#[async_trait]
impl ManagementSpanTransport for NoEffects {
    type Error = Infallible;

    async fn success_hook(
        &self,
        _payload: &ManagementPayload,
        _parent_otel_span: Option<&LiveSpan>,
    ) -> Result<(), Infallible> {
        Ok(())
    }

    async fn failure_hook(
        &self,
        _payload: &ManagementPayload,
        _parent_otel_span: Option<&LiveSpan>,
    ) -> Result<(), Infallible> {
        Ok(())
    }
}

impl ManagementCacheEviction for NoEffects {
    type Error = Infallible;

    fn delete_cache(&self, _key: &str) -> Result<(), Infallible> {
        Ok(())
    }
}

mod tests;
