pub mod answer;
pub mod descriptors;
mod estimate;
mod fetch_params;
pub mod fields;
pub mod input;
pub mod limiter;
pub mod routing;
mod skip;

mod tests;

use std::error::Error;
use std::sync::atomic::AtomicBool;

use betterlite_auth::resolved_key::auth::{AuthError, AuthInput, UserAPIKeyAuth};
use betterlite_config::sections::batch_limit_descriptors::Settings;

use crate::api::api_boundaries::{ApiBoundariesInput, ApiLimitError, ApiLimits, ItemsBoundary};
use crate::api::model_credentials::CredentialSource;
use answer::{BatchLimitAnswer, BatchLimitDescriptorsResult};
use fields::BatchPrincipalFields;
use input::{BatchLimitCall, BatchLimitDescriptorsInput};
use limiter::BatchLimiterSource;
use routing::BatchCredentialError;

// Every failure this slice raises itself; a router failure that is not an HTTPException stays distinct
#[derive(Debug, thiserror::Error)]
pub enum BatchLimitError<LimiterError, CredentialError>
where
    LimiterError: Error + 'static,
    CredentialError: Error + 'static,
{
    #[error(transparent)]
    Principal(#[from] AuthError),
    #[error("failed to read the batch principal fields")]
    Fields(#[source] serde_json::Error),
    #[error(transparent)]
    Boundary(#[from] ApiLimitError),
    #[error(transparent)]
    Limits(LimiterError),
    #[error(transparent)]
    Credentials(#[from] BatchCredentialError<CredentialError>),
}

pub struct BatchLimitDescriptors<Limiter, Credentials> {
    pub settings: Settings,
    pub limiter: Limiter,
    pub credentials: Option<Credentials>,
    pub limits: ApiLimits,
    pub warned_unsupported_model_skip: AtomicBool,
}

impl<Limiter, Credentials> BatchLimitDescriptors<Limiter, Credentials>
where
    Limiter: BatchLimiterSource,
    Credentials: CredentialSource,
{
    pub fn run(
        &self,
        input: &BatchLimitDescriptorsInput,
    ) -> Result<BatchLimitDescriptorsResult, BatchLimitError<Limiter::Error, Credentials::Error>>
    {
        self.limits.check(ApiBoundariesInput::Items(ItemsBoundary {
            known_count: input.calls.len(),
        }))?;
        let auth = UserAPIKeyAuth::construct(AuthInput::Dict(input.principal.0.clone()))?;
        let fields: BatchPrincipalFields = serde_json::to_value(&auth)
            .and_then(serde_json::from_value)
            .map_err(BatchLimitError::Fields)?;
        let mut answers = Vec::with_capacity(input.calls.len());
        for call in &input.calls {
            answers.push(self.answer(&auth, &fields, call)?);
        }
        Ok(BatchLimitDescriptorsResult { answers })
    }

    #[expect(clippy::too_many_lines, reason = "one arm per owned batch callable")]
    fn answer(
        &self,
        auth: &UserAPIKeyAuth,
        fields: &BatchPrincipalFields,
        call: &BatchLimitCall,
    ) -> Result<BatchLimitAnswer, BatchLimitError<Limiter::Error, Credentials::Error>> {
        match call {
            BatchLimitCall::FileBoundModel { data } => Ok(BatchLimitAnswer::FileBoundModel {
                model: routing::file_bound_model(data),
            }),
            BatchLimitCall::BatchRoutingModel { data } => Ok(BatchLimitAnswer::BatchRoutingModel {
                model: routing::batch_routing_model(data),
            }),
            BatchLimitCall::ResolveBatchProvider { model } => {
                Ok(BatchLimitAnswer::ResolveBatchProvider {
                    provider: routing::resolve_batch_provider(self, model.as_deref())?,
                })
            }
            BatchLimitCall::CreateRateLimitDescriptors { data } => {
                Ok(BatchLimitAnswer::CreateRateLimitDescriptors {
                    descriptors: descriptors::create_batch_descriptors(self, fields, auth, data)
                        .map_err(BatchLimitError::Limits)?,
                })
            }
            BatchLimitCall::ProjectHasIoLimits => Ok(BatchLimitAnswer::ProjectHasIoLimits {
                has_limits: descriptors::project_has_io_limits(auth),
            }),
            BatchLimitCall::CreateProjectIoDescriptors { per_model_usage } => {
                let (descriptors, increments) =
                    descriptors::project_io_descriptors(self, auth, per_model_usage);
                Ok(BatchLimitAnswer::CreateProjectIoDescriptors {
                    descriptors,
                    increments,
                })
            }
            BatchLimitCall::ShouldSkip {
                data,
                has_enqueued_scopes,
            } => {
                let decision = skip::should_skip(self, fields, auth, data, *has_enqueued_scopes)?;
                Ok(BatchLimitAnswer::ShouldSkip {
                    skip: decision.skip,
                    descriptors: decision.descriptors,
                })
            }
            BatchLimitCall::KeyRequiresAccessCheck => {
                Ok(BatchLimitAnswer::KeyRequiresAccessCheck {
                    required: descriptors::key_requires_access_check(fields),
                })
            }
            BatchLimitCall::EstimateOutputTokens {
                entry,
                min_configured_otpm_limit,
            } => Ok(BatchLimitAnswer::EstimateOutputTokens {
                tokens: estimate::estimate_output_tokens(self, entry, *min_configured_otpm_limit),
            }),
            BatchLimitCall::HasApplicableLimits {
                descriptors: requested,
            } => Ok(BatchLimitAnswer::HasApplicableLimits {
                applicable: descriptors::has_applicable_limits(requested),
            }),
            BatchLimitCall::ResolveFetchParams {
                file_id,
                custom_llm_provider,
                data,
            } => {
                let (resolved_file_id, params) =
                    fetch_params::fetch_params(self, file_id, custom_llm_provider, data)?;
                Ok(BatchLimitAnswer::ResolveFetchParams {
                    file_id: resolved_file_id,
                    params,
                })
            }
        }
    }
}
