use axum::body::Bytes;
use betterlite_auth::auth_request_policy::ip::RequestProvenance;
use betterlite_auth::auth_request_policy::policy::{PreDbRequest, pre_db_read_auth_checks};
use betterlite_auth::request_body_safety::RequestBodySafety;
use betterlite_error::proxy_exception::ProxyException;

use crate::auth::auth_identity::AuthContext;
use crate::auth::request_body_safety::{CoreMetadata, request_model, request_object};
use crate::state::auth_request_policy::Slot;

mod tests;

#[derive(Clone, Debug)]
pub struct AuthRequestPolicyStageInput {
    pub context: AuthContext,
    pub content_length: Option<String>,
    pub forwarded_for: Option<String>,
    pub client_host: Option<String>,
    pub route: String,
    pub content_type: Option<String>,
    pub body: Bytes,
    pub slot: Option<Slot>,
}

pub(crate) fn run_stage(
    stage: &AuthRequestPolicyStageInput,
) -> Result<AuthContext, Box<ProxyException>> {
    let Some(slot) = &stage.slot else {
        return Ok(stage.context.clone());
    };
    let content_type = stage.content_type.as_deref().unwrap_or_default();
    let body = request_object(&stage.body, content_type);
    let model = body.as_ref().map_or("", request_model);
    let safety = body.as_ref().map(|object| RequestBodySafety {
        body: object,
        model,
        allow_client_side_credentials: slot.body_safety.settings.allow_client_side_credentials,
        allowed_hosts: &slot.body_safety.settings.allowed_hosts,
        grants: slot,
        metadata: &CoreMetadata,
        max_fallbacks: slot.body_safety.settings.max_fallbacks,
    });
    let request = PreDbRequest {
        content_length: stage.content_length.as_deref(),
        body: &stage.body,
        route: stage.route.as_str(),
        provenance: RequestProvenance {
            forwarded_for: stage.forwarded_for.as_deref(),
            client_host: stage.client_host.as_deref(),
        },
        body_safety: safety,
    };
    pre_db_read_auth_checks(request, &slot.settings)
        .map_err(|error| Box::new(error.into_proxy_exception()))?;
    Ok(stage.context.clone())
}
