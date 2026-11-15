use std::collections::BTreeMap;

use betterlite_auth::auth_limit_inputs::account::account_limits;
use betterlite_auth::auth_limit_inputs::error::AuthLimitInputsError;
use betterlite_auth::auth_limit_inputs::key_limits::{
    LimitQuery, ModelRateLimitKey, get_key_model_rpm_limit, get_key_model_tpm_limit,
    get_key_own_model_rate_limit,
};
use betterlite_auth::auth_limit_inputs::metadata_limits::{
    MetadataAccessor, model_limit_from_metadata,
};
use betterlite_auth::resolved_key::auth::{AuthError, AuthInput, UserAPIKeyAuth};
use betterlite_db::budget_permission_rows::JsonData;

use crate::limiter_records::{
    ModelGroupResolver, PROJECT_ITPM_DESCRIPTOR_KEY, PROJECT_OTPM_DESCRIPTOR_KEY,
    RateLimitDescriptor, RateLimitDescriptorRateLimit, RateLimitedModel,
};

mod tests;

// LiteLLM builds descriptor values with an f-string, so an absent id lands in the counter name as this text
const NONE_TEXT: &str = "None";

#[derive(Debug, thiserror::Error)]
pub enum ModelDescriptorsError {
    #[error(transparent)]
    KeyLimits(#[from] AuthLimitInputsError),
    #[error(transparent)]
    RequestMetadata(#[from] AuthError),
}

pub struct ModelDescriptorBuilder {
    pub window_size: i64,
    pub resolver: Box<dyn ModelGroupResolver>,
}

fn metadata_limits(
    auth: &UserAPIKeyAuth,
    accessor: MetadataAccessor,
    model: &RateLimitedModel,
    kinds: (ModelRateLimitKey, ModelRateLimitKey),
) -> (Option<i64>, Option<i64>) {
    (
        model.limit_in(model_limit_from_metadata(auth, accessor, kinds.0).as_ref()),
        model.limit_in(model_limit_from_metadata(auth, accessor, kinds.1).as_ref()),
    )
}

fn inherited_team_limit(
    auth: &UserAPIKeyAuth,
    model: &RateLimitedModel,
    kind: ModelRateLimitKey,
) -> Option<i64> {
    let team_limit =
        model.limit_in(model_limit_from_metadata(auth, MetadataAccessor::Team, kind).as_ref());
    let key_owns_limit = model
        .limit_in(get_key_own_model_rate_limit(auth, kind).as_ref())
        .is_some();
    if key_owns_limit { None } else { team_limit }
}

#[derive(Clone, Copy)]
struct ScopeDescriptor<'scope> {
    key: &'scope str,
    id: Option<&'scope str>,
    limits: (Option<i64>, Option<i64>),
}

impl ModelDescriptorBuilder {
    #[must_use]
    pub fn rated_model(&self, requested: Option<&str>) -> Option<RateLimitedModel> {
        let wanted = requested.filter(|name| !name.is_empty())?;
        let group = self
            .resolver
            .resolve(wanted)
            .filter(|group| !group.is_empty())
            .unwrap_or_else(|| wanted.to_owned());
        Some(RateLimitedModel {
            requested: wanted.to_owned(),
            group,
        })
    }

    fn descriptor(
        &self,
        key: &str,
        value: String,
        requests_per_unit: Option<i64>,
        tokens_per_unit: Option<i64>,
    ) -> RateLimitDescriptor {
        RateLimitDescriptor {
            key: key.to_owned(),
            value,
            rate_limit: Some(RateLimitDescriptorRateLimit {
                requests_per_unit,
                tokens_per_unit,
                max_parallel_requests: None,
                window_size: Some(self.window_size),
            }),
        }
    }

    fn push_model_descriptor(
        &self,
        out: &mut Vec<RateLimitDescriptor>,
        model: &RateLimitedModel,
        descriptor: ScopeDescriptor<'_>,
    ) {
        if descriptor.limits.0.is_none() && descriptor.limits.1.is_none() {
            return;
        }
        let value = format!("{}:{}", descriptor.id.unwrap_or(NONE_TEXT), model.group);
        out.push(self.descriptor(
            descriptor.key,
            value,
            descriptor.limits.0,
            descriptor.limits.1,
        ));
    }

    #[must_use]
    pub fn organization_descriptors(
        &self,
        auth: &UserAPIKeyAuth,
        requested_model: Option<&str>,
    ) -> Vec<RateLimitDescriptor> {
        let account = account_limits(auth);
        let mut descriptors = Vec::new();
        if let Some(org_id) = account.org_id
            && (account.organization_rpm_limit.is_some()
                || account.organization_tpm_limit.is_some())
        {
            descriptors.push(self.descriptor(
                "organization",
                org_id.to_owned(),
                account.organization_rpm_limit,
                account.organization_tpm_limit,
            ));
        }
        if let Some(model) = self.rated_model(requested_model) {
            let (requests, tokens) = metadata_limits(
                auth,
                MetadataAccessor::Organization,
                &model,
                (ModelRateLimitKey::Rpm, ModelRateLimitKey::Tpm),
            );
            self.push_model_descriptor(
                &mut descriptors,
                &model,
                ScopeDescriptor {
                    key: "model_per_organization",
                    id: account.org_id,
                    limits: (requests, tokens),
                },
            );
        }
        descriptors
    }

    pub fn key_model_descriptor(
        &self,
        auth: &UserAPIKeyAuth,
        requested_model: Option<&str>,
        out: &mut Vec<RateLimitDescriptor>,
    ) -> Result<(), ModelDescriptorsError> {
        let Some(model) = self.rated_model(requested_model) else {
            return Ok(());
        };
        let query = LimitQuery {
            model_name: Some(model.group.as_str()),
            catalog: None,
        };
        let tokens = model.limit_in(get_key_model_tpm_limit(auth, &query)?.as_ref());
        let requests = model.limit_in(get_key_model_rpm_limit(auth, &query)?.as_ref());
        self.push_model_descriptor(
            out,
            &model,
            ScopeDescriptor {
                key: "model_per_key",
                id: account_limits(auth).hashed_api_key,
                limits: (requests, tokens),
            },
        );
        Ok(())
    }

    pub fn team_model_descriptor(
        &self,
        auth: &UserAPIKeyAuth,
        requested_model: Option<&str>,
        out: &mut Vec<RateLimitDescriptor>,
    ) {
        let Some(model) = self.rated_model(requested_model) else {
            return;
        };
        let mut limits = [None, None];
        for (slot, kind) in limits
            .iter_mut()
            .zip([ModelRateLimitKey::Rpm, ModelRateLimitKey::Tpm])
        {
            *slot = inherited_team_limit(auth, &model, kind);
        }
        let [requests, tokens] = limits;
        self.push_model_descriptor(
            out,
            &model,
            ScopeDescriptor {
                key: "model_per_team",
                id: account_limits(auth).team_id,
                limits: (requests, tokens),
            },
        );
    }

    pub fn project_model_descriptor(
        &self,
        auth: &UserAPIKeyAuth,
        requested_model: Option<&str>,
        out: &mut Vec<RateLimitDescriptor>,
    ) {
        if let Some(model) = self.rated_model(requested_model) {
            let (requests, tokens) = metadata_limits(
                auth,
                MetadataAccessor::Project,
                &model,
                (ModelRateLimitKey::Rpm, ModelRateLimitKey::Tpm),
            );
            self.push_model_descriptor(
                out,
                &model,
                ScopeDescriptor {
                    key: "model_per_project",
                    id: account_limits(auth).project_id,
                    limits: (requests, tokens),
                },
            );
        }
    }

    pub fn project_io_descriptors(
        &self,
        auth: &UserAPIKeyAuth,
        requested_model: Option<&str>,
        out: &mut Vec<RateLimitDescriptor>,
    ) {
        let Some(model) = self.rated_model(requested_model) else {
            return;
        };
        let Some(project_id) = account_limits(auth).project_id else {
            return;
        };
        let (input_tokens, output_tokens) = metadata_limits(
            auth,
            MetadataAccessor::Project,
            &model,
            (ModelRateLimitKey::Itpm, ModelRateLimitKey::Otpm),
        );
        for (key, limit) in [
            (PROJECT_ITPM_DESCRIPTOR_KEY, input_tokens),
            (PROJECT_OTPM_DESCRIPTOR_KEY, output_tokens),
        ] {
            self.push_model_descriptor(
                out,
                &model,
                ScopeDescriptor {
                    key,
                    id: Some(project_id),
                    limits: (None, limit),
                },
            );
        }
    }

    // ast-grep-ignore: no-long-names
    pub fn key_owns_tpm_from_request_metadata(
        request_metadata: &BTreeMap<String, JsonData>,
        model: Option<&RateLimitedModel>,
    ) -> Result<bool, ModelDescriptorsError> {
        let Some(rated) = model else {
            return Ok(false);
        };
        let mut fields = BTreeMap::new();
        for (field, source) in [
            ("metadata", "user_api_key_metadata"),
            ("model_max_budget", "user_api_key_model_max_budget"),
        ] {
            let value = match request_metadata.get(source) {
                None | Some(JsonData::Null) => JsonData::Object(BTreeMap::new()),
                Some(present) => present.clone(),
            };
            fields.insert(field.to_owned(), value);
        }
        let key_view = UserAPIKeyAuth::construct(AuthInput::Dict(fields))?;
        Ok(rated
            .limit_in(get_key_own_model_rate_limit(&key_view, ModelRateLimitKey::Tpm).as_ref())
            .is_some())
    }
}
