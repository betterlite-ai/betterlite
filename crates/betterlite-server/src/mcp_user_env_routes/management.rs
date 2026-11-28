use std::future::Future;
use std::sync::Arc;

use axum::Extension;
use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use betterlite_config::sections::request_body::Settings as RequestBodySettings;
use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_mcp::registry::resolution::ServerAccessCheck;
use betterlite_models::model_registration::JsonObject;
use serde::Serialize;

use crate::auth::auth_identity::AuthContext;
use crate::management_effects::wrapper::{
    ManagementCall, ManagementDeps, ManagementFailure, NoEffects, SystemClock,
    default_management_auth, management_endpoint_wrapper,
};
use crate::management_payload::otel_span::ManagementSpanDeps;
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
        let auth =
            default_management_auth(source_key.as_ref()).map_err(UserEnvRouteError::DefaultAuth)?;
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

mod tests;
