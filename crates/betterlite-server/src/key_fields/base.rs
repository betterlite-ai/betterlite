use std::collections::BTreeMap;

use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::identity_rows::Payload;
use betterlite_db::team_rows::member::BudgetLimitEntry;

use super::decode::{KeyProjection, TypedValue, blank_to_null, decode_group, project, to_object};
use super::defaults::{BASE_DEFAULTS, KEY_DEFAULTS};
use super::error::DecodeError;
use super::values::{AllowedVectorStoreIndexItem, Amount, LimitType, LiteLLMObjectPermissionBase};

nullable_record! {
    (Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)
    pub struct GenerateRequestBase {
        key_alias: String,
        duration: String,
        models: Vec<JsonData>,
        spend: Amount,
        max_budget: Amount,
        user_id: String,
        team_id: String,
        agent_id: String,
        max_parallel_requests: i64,
        metadata: Payload,
        tpm_limit: i64,
        rpm_limit: i64,
        budget_duration: String,
        budget_limits: Vec<BudgetLimitEntry>,
        allowed_cache_controls: Vec<JsonData>,
        config: Payload,
        permissions: Payload,
        model_max_budget: Payload,
        budget_fallbacks: BTreeMap<String, Vec<String>>,
        model_rpm_limit: Payload,
        model_tpm_limit: Payload,
        mcp_rpm_limit: BTreeMap<String, i64>,
        tag_rpm_limit: BTreeMap<String, i64>,
        guardrails: Vec<String>,
        policies: Vec<String>,
        prompts: Vec<String>,
        blocked: bool,
        aliases: Payload,
        object_permission: LiteLLMObjectPermissionBase,
    }
}

nullable_record! {
    (Clone, serde::Serialize, serde::Deserialize)
    pub struct KeyRequestFields {
        tpd_limit: i64,
        #[serde(rename = "default_estimated_output_tokens")]
        estimated_output_tokens: i64,
        #[serde(rename = "default_estimated_output_tokens_per_model")]
        per_model_output_tokens: BTreeMap<String, i64>,
        budget_id: String,
        end_user_budget_id: String,
        tags: Vec<String>,
        disable_global_guardrails: bool,
        enable_prompt_caching: bool,
        throttle_on_budget_exceeded: bool,
        enforced_params: Vec<String>,
        allowed_routes: Vec<JsonData>,
        allowed_passthrough_routes: Vec<JsonData>,
        allowed_vector_store_indexes: Vec<AllowedVectorStoreIndexItem>,
        rpm_limit_type: LimitType,
        tpm_limit_type: LimitType,
        router_settings: Payload,
        access_group_ids: Vec<String>,
    } secret { key }
}

impl GenerateRequestBase {
    pub fn read(value: &TypedValue) -> Result<Self, DecodeError> {
        let mut fields = value.object()?;
        blank_to_null(&mut fields, &["max_budget"]);
        decode_group(&fields)
    }

    pub fn project(&self) -> Result<KeyProjection, DecodeError> {
        let fields = to_object(self)?;
        project(
            &[BASE_DEFAULTS],
            &[&fields],
            self.object_permission.as_ref().and_then(Option::as_ref),
        )
    }
}

#[derive(Clone)]
pub struct KeyRequestBase {
    pub base: GenerateRequestBase,
    pub key_fields: KeyRequestFields,
}

impl KeyRequestBase {
    pub(crate) fn decode(fields: &BTreeMap<String, JsonData>) -> Result<Self, DecodeError> {
        let request = Self {
            base: decode_group(fields)?,
            key_fields: decode_group(fields)?,
        };
        request.check_positive_tokens()?;
        Ok(request)
    }

    pub fn read(value: &TypedValue) -> Result<Self, DecodeError> {
        let mut fields = value.object()?;
        blank_to_null(&mut fields, &["max_budget"]);
        Self::decode(&fields)
    }

    pub fn project(&self) -> Result<KeyProjection, DecodeError> {
        let mut projection = self.base.project()?;
        let key = to_object(&self.key_fields)?;
        let mut resolved = super::decode::resolved(&[KEY_DEFAULTS], &[])?;
        resolved.extend(projection.resolved);
        resolved.extend(key.clone());
        projection.supplied.extend(key);
        Ok(KeyProjection {
            supplied: projection.supplied,
            resolved,
        })
    }

    pub(crate) fn object_permission(&self) -> Option<&LiteLLMObjectPermissionBase> {
        self.base
            .object_permission
            .as_ref()
            .and_then(Option::as_ref)
    }

    // PositiveInt rejects zero and negatives for both the single and the per-model estimate.
    fn check_positive_tokens(&self) -> Result<(), DecodeError> {
        if matches!(self.key_fields.estimated_output_tokens, Some(Some(value)) if value <= 0) {
            return Err(DecodeError::NotPositive {
                field: "default_estimated_output_tokens".to_owned(),
            });
        }
        let invalid = self
            .key_fields
            .per_model_output_tokens
            .as_ref()
            .and_then(Option::as_ref)
            .and_then(|per_model| per_model.iter().find(|(_, value)| **value <= 0));
        if let Some((model, _)) = invalid {
            return Err(DecodeError::NotPositive {
                field: format!("default_estimated_output_tokens_per_model.{model}"),
            });
        }
        Ok(())
    }
}
