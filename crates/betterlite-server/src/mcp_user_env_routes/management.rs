use std::future::Future;

use axum::Extension;
use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use betterlite_config::sections::request_body::Settings as RequestBodySettings;
use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_mcp::registry::resolution::ServerAccessCheck;
use betterlite_models::model_registration::JsonObject;
use serde::Serialize;

use crate::auth::auth_identity::AuthContext;
use crate::management_effects::wrapper::{
    ManagementCall, ManagementFailure, NoEffects, default_management_auth,
    management_endpoint_wrapper, no_effect_deps,
};
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
        let deps = no_effect_deps(&body_settings, &limits, &effects, &auth);
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

mod tests;
