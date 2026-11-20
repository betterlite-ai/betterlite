use betterlite_auth::request_quota_view::RequestQuotaViewExt as _;
use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use betterlite_core::api::request_destination::RequestData;

use super::RequestScopeBuilder;
use super::body_text;
use super::key_scopes::{KeyScopesError, ScopeInputs, create_rate_limit_descriptors};
use crate::identity_descriptors::tag_create::create_tag_descriptors;
use crate::limiter_records::RateLimitDescriptor;

mod tests;

#[derive(Debug, thiserror::Error)]
pub enum RequestAssemblyError {
    #[error(transparent)]
    KeyScopes(#[from] KeyScopesError),
}

// ast-grep-ignore: no-long-names
pub async fn build_request_rate_limit_descriptors(
    builder: &RequestScopeBuilder<'_>,
    auth: &UserAPIKeyAuth,
    data: &RequestData,
    call_type: Option<&str>,
) -> Result<Vec<RateLimitDescriptor>, RequestAssemblyError> {
    let view = auth.request_quota_view();
    let model = body_text(data, "model");
    let dynamic = view.rpm_limit_type == Some("dynamic") || view.tpm_limit_type == Some("dynamic");
    let model_has_failures = match model.filter(|name| !name.is_empty()) {
        Some(name) if dynamic => builder.health.model_has_recent_failures(name).await,
        Some(_) | None => false,
    };
    let mut descriptors = create_rate_limit_descriptors(
        builder,
        ScopeInputs {
            auth,
            data,
            rpm_limit_type: view.rpm_limit_type,
            tpm_limit_type: view.tpm_limit_type,
            model_has_failures,
            call_type,
        },
    )?;
    builder
        .model
        .project_model_descriptor(auth, model, &mut descriptors);
    builder
        .model
        .project_io_descriptors(auth, model, &mut descriptors);
    descriptors.extend(builder.model.organization_descriptors(auth, model));
    descriptors
        .extend(create_tag_descriptors(data, builder.window_size, builder.tag_resolver).await);
    Ok(descriptors)
}
