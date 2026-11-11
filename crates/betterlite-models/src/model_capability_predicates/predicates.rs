use crate::decisions::Json;
use crate::lemonade_model_metadata::ModelInfoClient;
use crate::model_metadata_resolution::dynamic::DynamicMetadataSource;

use super::details::{self, CapabilityError, MetadataValue, Phrase, Strategy};
use super::raw;
use super::{CapabilityEnvironment, CapabilityPredicates, CapabilityRequest};

// LiteLLM's PROVIDERS_GLOBALLY_SUPPORT_RESPONSE_SCHEMA, as the provider slugs it compares.
const RESPONSE_SCHEMA_PROVIDERS: [&str; 5] = [
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
        match details::lookup(request, key, Strategy::DeclaredFirst, Phrase::Support, self).await {
            Ok((_, MetadataValue::Value(Json::Bool(true)))) => true,
            Ok((target, MetadataValue::Absent)) => details::fallback_answer(&target, key, self),
            Ok((_, MetadataValue::Value(_))) => false,
            Err(failure) => {
                details::provider_flag(&failure.model, failure.provider.as_deref(), key, self)
            }
        }
    }

    async fn declared_value_factory(
        &mut self,
        request: &CapabilityRequest<'_>,
        key: &str,
    ) -> Option<String> {
        let Ok((target, value)) =
            details::lookup(request, key, Strategy::Resolved, Phrase::Reading, self).await
        else {
            return None;
        };
        if let MetadataValue::Value(Json::Text(text)) = value {
            return Some(text);
        }
        match raw::bare_declaration(&target.model, key, self) {
            Some(Json::Text(text)) => Some(text),
            _ => None,
        }
    }

    async fn is_explicitly_disabled_factory(
        &mut self,
        request: &CapabilityRequest<'_>,
        key: &str,
    ) -> bool {
        let found = details::lookup(
            request,
            key,
            Strategy::DeclaredFirst,
            Phrase::DisabledState,
            self,
        )
        .await;
        let Ok((target, value)) = found else {
            return false;
        };
        if matches!(value, MetadataValue::Absent) {
            return matches!(
                raw::bare_declaration(&target.model, key, self),
                Some(Json::Bool(false))
            );
        }
        value == MetadataValue::Value(Json::Bool(false))
    }

    async fn supports_native_streaming(&mut self, request: &CapabilityRequest<'_>) -> Json {
        let Ok((_, value)) = details::lookup(
            request,
            "supports_native_streaming",
            Strategy::Resolved,
            Phrase::Support,
            self,
        )
        .await
        else {
            return Json::Bool(false);
        };
        // a missing or null flag reads as native streaming, and any other value passes through as written
        match value {
            MetadataValue::Absent => Json::Bool(true),
            MetadataValue::Value(declared) => declared,
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
        if RESPONSE_SCHEMA_PROVIDERS.contains(&target.provider.as_str()) {
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
