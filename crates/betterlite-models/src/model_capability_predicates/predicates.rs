use crate::decisions::Json;
use crate::lemonade_model_metadata::ModelInfoClient;
use crate::model_metadata_resolution::dynamic::DynamicMetadataSource;

use super::details::{self, CapabilityError, MetadataValue};
use super::{CapabilityEnvironment, CapabilityPredicates, CapabilityRequest};

// LiteLLM's PROVIDERS_GLOBALLY_SUPPORT_RESPONSE_SCHEMA, as the provider slugs it compares.
const GLOBALLY_RESPONSE_SCHEMA_PROVIDERS: [&str; 5] = [
    "predibase",
    "fireworks_ai",
    "lm_studio",
    "nebius",
    "databricks",
];

impl<Source, Transport> CapabilityPredicates for CapabilityEnvironment<'_, Source, Transport>
where
    Source: DynamicMetadataSource,
    Transport: ModelInfoClient,
{
    async fn supports_factory(&mut self, request: &CapabilityRequest<'_>, key: &str) -> bool {
        let target = match details::resolve_target(request, self) {
            Ok(target) => target,
            Err(error) => {
                details::log_failure(
                    &format!("checking {key} support"),
                    request.model,
                    request.custom_llm_provider,
                    &CapabilityError::from(error),
                );
                return details::provider_flag(
                    request.model,
                    request.custom_llm_provider,
                    key,
                    self,
                );
            }
        };
        let value = match details::metadata_field(&target, key, self).await {
            Ok(value) => value,
            Err(error) => {
                details::log_failure(
                    &format!("checking {key} support"),
                    &target.model,
                    Some(&target.provider),
                    &error,
                );
                return details::provider_flag(&target.model, Some(&target.provider), key, self);
            }
        };
        match value {
            MetadataValue::Value(Json::Bool(true)) => true,
            MetadataValue::Value(_) => false,
            MetadataValue::Absent => {
                details::bare_is_true(&target.model, key, self)
                    || details::provider_flag(&target.model, Some(&target.provider), key, self)
            }
        }
    }

    async fn declared_value_factory(
        &mut self,
        request: &CapabilityRequest<'_>,
        key: &str,
    ) -> Option<String> {
        let target = match details::resolved_target(request, self) {
            Ok(target) => target,
            Err(error) => {
                details::log_failure(
                    &format!("reading {key}"),
                    request.model,
                    request.custom_llm_provider,
                    &CapabilityError::from(error),
                );
                return None;
            }
        };
        let value = match details::metadata_field(&target, key, self).await {
            Ok(value) => value,
            Err(error) => {
                details::log_failure(
                    &format!("reading {key}"),
                    &target.model,
                    Some(&target.provider),
                    &error,
                );
                return None;
            }
        };
        if let MetadataValue::Value(Json::Text(text)) = value {
            return Some(text);
        }
        details::bare_text(&target.model, key, self)
    }

    async fn is_explicitly_disabled_factory(
        &mut self,
        request: &CapabilityRequest<'_>,
        key: &str,
    ) -> bool {
        let target = match details::resolve_target(request, self) {
            Ok(target) => target,
            Err(error) => {
                details::log_failure(
                    &format!("checking {key} disabled state"),
                    request.model,
                    request.custom_llm_provider,
                    &CapabilityError::from(error),
                );
                return false;
            }
        };
        let value = match details::metadata_field(&target, key, self).await {
            Ok(value) => value,
            Err(error) => {
                details::log_failure(
                    &format!("checking {key} disabled state"),
                    &target.model,
                    Some(&target.provider),
                    &error,
                );
                return false;
            }
        };
        match value {
            MetadataValue::Value(Json::Bool(false)) => true,
            MetadataValue::Absent => details::bare_is_false(&target.model, key, self),
            MetadataValue::Value(_) => false,
        }
    }

    async fn supports_native_streaming(&mut self, request: &CapabilityRequest<'_>) -> Json {
        let target = match details::resolved_target(request, self) {
            Ok(target) => target,
            Err(error) => {
                details::log_failure(
                    "checking supports_native_streaming support",
                    request.model,
                    request.custom_llm_provider,
                    &CapabilityError::from(error),
                );
                return Json::Bool(false);
            }
        };
        match details::metadata_field(&target, "supports_native_streaming", self).await {
            // a missing or null flag reads as native streaming, and any other value passes through as written
            Ok(MetadataValue::Absent) => Json::Bool(true),
            Ok(MetadataValue::Value(value)) => value,
            Err(error) => {
                details::log_failure(
                    "checking supports_native_streaming support",
                    &target.model,
                    Some(&target.provider),
                    &error,
                );
                Json::Bool(false)
            }
        }
    }

    async fn supports_response_schema(&mut self, request: &CapabilityRequest<'_>) -> bool {
        let target = match details::resolved_target(request, self) {
            Ok(target) => target,
            Err(error) => {
                details::log_failure(
                    "checking response schema support",
                    request.model,
                    request.custom_llm_provider,
                    &CapabilityError::from(error),
                );
                return false;
            }
        };
        if GLOBALLY_RESPONSE_SCHEMA_PROVIDERS.contains(&target.provider.as_str()) {
            return true;
        }
        let resolved = CapabilityRequest {
            model: &target.model,
            custom_llm_provider: Some(&target.provider),
        };
        self.supports_factory(&resolved, "supports_response_schema")
            .await
    }
}
