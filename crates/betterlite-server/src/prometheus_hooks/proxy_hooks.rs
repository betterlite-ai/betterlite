use std::future::{Future, ready};
use std::time::Instant;

use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use betterlite_core::api::request_principal_metadata::PrincipalFacts;
use betterlite_core::api::request_principal_metadata::principal::logged_api_key;
use betterlite_logging::logging_metadata::params::LitellmParams;
use betterlite_logging::logging_metadata::tags::{ProxyHeaders, TagSettings, get_request_tags};
use betterlite_logging::prometheus::failure::RequestedModelCatalog;
use betterlite_logging::prometheus::failure::exception::LabelException;
use betterlite_logging::prometheus::failure::exception_name::exception_class_name;
use betterlite_logging::prometheus::failure::model_cardinality::bounded_requested_model_label;
use betterlite_logging::prometheus::failure::provider::{
    ProviderRequestData, api_provider_from_exception, api_provider_from_request_data,
};
use betterlite_logging::prometheus::failure::rate_limit::extract_rate_limit_labels;
use betterlite_logging::prometheus::failure::status_code::extract_status_code;
use betterlite_logging::prometheus_label_projection::label_factory_context::PrometheusLabelFactoryContext;
use betterlite_logging::prometheus_label_projection::prometheus_label_factory::LabelSource;
use betterlite_models::decisions::Json;
use betterlite_models::resolution::ResolutionContext;
use betterlite_openai::openai_base_config::Object as JsonMap;
use betterlite_prometheus::collector_factory::collector::MetricValue;
use betterlite_prometheus::label_value_record::{LabelDumpValue, UserApiKeyLabelValues};
use indexmap::IndexMap;
use rust_decimal::Decimal;

use super::error::PrometheusHookError;
use super::hooks::PrometheusHooks;
use super::json::{as_object, as_text, metadata_text, request_json};
use super::labels::text_tags;
use super::metrics::CounterWrite;

impl PrometheusHooks {
    #[must_use]
    pub fn async_post_call_success_hook() -> impl Future<Output = ()> + Send {
        // the total requests counter is counted in the success event, so this hook stays empty
        ready(())
    }

    #[must_use]
    pub fn async_post_call_failure_hook<'hook>(
        &'hook self,
        request_data: &'hook JsonMap,
        exception: &'hook LabelException,
        auth: &'hook UserAPIKeyAuth,
    ) -> impl Future<Output = ()> + Send + 'hook {
        if let Err(error) = self.record_proxy_failure(request_data, exception, auth) {
            tracing::error!("prometheus Layer Error(): Exception occurred - {error}");
        }
        ready(())
    }

    fn record_proxy_failure(
        &self,
        request_data: &JsonMap,
        exception: &LabelException,
        auth: &UserAPIKeyAuth,
    ) -> Result<(), PrometheusHookError> {
        let data = request_json(request_data);
        let fields = as_object(&data);
        let principal: PrincipalFacts = serde_json::to_value(auth)
            .and_then(serde_json::from_value)
            .map_err(PrometheusHookError::Principal)?;
        let entries = self.proxy_entries(fields, exception, &principal)?;
        let values = UserApiKeyLabelValues::from_entries(&entries);
        let context = PrometheusLabelFactoryContext::new(
            &values,
            &self.settings.labels,
            self.settings.end_user,
        )?;
        let metrics = self.metrics();
        let now = Instant::now();
        for (counter, name) in [
            (
                &self.collectors.litellm_proxy_failed_requests_metric, // ast-grep-ignore: no-long-names
                "litellm_proxy_failed_requests_metric",
            ),
            (
                &self.collectors.litellm_proxy_total_requests_metric, // ast-grep-ignore: no-long-names
                "litellm_proxy_total_requests_metric",
            ),
        ] {
            metrics.emit(CounterWrite {
                counter,
                metric_name: name,
                values: &values,
                source: LabelSource::Context(&context),
                amount: MetricValue::Finite(Decimal::ONE),
                now,
            })?;
        }
        Ok(())
    }

    fn proxy_entries(
        &self,
        fields: Option<&IndexMap<String, Json>>,
        exception: &LabelException,
        principal: &PrincipalFacts,
    ) -> Result<Vec<(String, LabelDumpValue)>, PrometheusHookError> {
        let status_code = extract_status_code(None, Some(exception), None);
        let tags = self.proxy_tags(fields)?;
        let metadata = fields
            .and_then(|request_fields| request_fields.get("metadata"))
            .and_then(as_object);
        let (rate_limit_category, rate_limit_type) = extract_rate_limit_labels(Some(exception));
        let hashed_api_key = match status_code {
            Some(401) => None,
            Some(_) | None => logged_api_key(principal).map(str::to_owned),
        };
        let status_text = status_code.map_or_else(|| "None".to_owned(), |code| code.to_string());
        let requested = fields
            .and_then(|request_fields| request_fields.get("model"))
            .cloned()
            .unwrap_or_else(|| Json::Text(String::new()));
        let mut entries = Vec::new();
        push_value(&mut entries, "end_user", principal.end_user_id.clone());
        push_value(&mut entries, "user", principal.user_id.clone());
        push_value(&mut entries, "user_email", principal.user_email.clone());
        push_value(&mut entries, "hashed_api_key", hashed_api_key);
        push_value(&mut entries, "api_key_alias", principal.key_alias.clone());
        push_value(&mut entries, "team", principal.team_id.clone());
        push_value(&mut entries, "team_alias", principal.team_alias.clone());
        push_value(&mut entries, "org_id", principal.org_id.clone());
        push_value(
            &mut entries,
            "org_alias",
            principal.organization_alias.clone(),
        );
        push_value(
            &mut entries,
            "requested_model",
            self.proxy_requested_model(&requested),
        );
        push_value(&mut entries, "status_code", Some(status_text.clone()));
        push_value(&mut entries, "exception_status", Some(status_text));
        push_value(
            &mut entries,
            "exception_class",
            Some(exception_class_name(exception)),
        );
        push_value(&mut entries, "rate_limit_category", rate_limit_category);
        push_value(&mut entries, "rate_limit_type", rate_limit_type);
        entries.push(("tags".to_owned(), LabelDumpValue::Strings(text_tags(&tags))));
        push_value(&mut entries, "route", principal.request_route.clone());
        push_value(
            &mut entries,
            "client_ip",
            metadata_text(metadata, "requester_ip_address"),
        );
        push_value(
            &mut entries,
            "user_agent",
            metadata_text(metadata, "user_agent"),
        );
        push_value(&mut entries, "model_id", proxy_model_id(metadata, fields));
        push_value(
            &mut entries,
            "api_provider",
            self.proxy_api_provider(fields, exception),
        );
        if self.settings.labels.emit_stream_label {
            push_value(&mut entries, "stream", Some(proxy_stream_text(fields)));
        }
        Ok(entries)
    }

    fn proxy_requested_model(&self, requested: &Json) -> Option<String> {
        let catalog = self.catalog();
        bounded_requested_model_label(
            Some(requested),
            false,
            catalog
                .as_ref()
                .map(|adapter| adapter as &dyn RequestedModelCatalog),
        )
    }

    fn proxy_api_provider(
        &self,
        fields: Option<&IndexMap<String, Json>>,
        exception: &LabelException,
    ) -> Option<String> {
        let litellm_params = fields
            .and_then(|request_fields| request_fields.get("litellm_params"))
            .and_then(as_object);
        let standard_logging = fields
            .and_then(|request_fields| request_fields.get("standard_logging_object"))
            .and_then(as_object);
        let model = litellm_params
            .and_then(|params| params.get("model"))
            .and_then(as_text)
            .filter(|model| !model.is_empty())
            .or_else(|| {
                fields
                    .and_then(|request_fields| request_fields.get("model"))
                    .and_then(as_text)
                    .filter(|model| !model.is_empty())
            });
        let request = ProviderRequestData {
            litellm_params,
            standard_logging,
            model,
        };
        let context = ResolutionContext {
            catalog: self.provider.catalog.as_ref(),
            known_model_names: &self.provider.known_model_names,
            custom_providers: &self.provider.custom_providers,
            environment: &self.provider.environment,
            registry: self.provider.registry.clone(),
            policies: self.provider.policies.clone(),
            globals: self.provider.globals.clone(),
        };
        api_provider_from_request_data(&request, &context)
            .or_else(|| api_provider_from_exception(exception))
    }

    fn proxy_tags(
        &self,
        fields: Option<&IndexMap<String, Json>>,
    ) -> Result<Vec<Json>, PrometheusHookError> {
        let params = LitellmParams {
            metadata: object_field(fields, "metadata").cloned(),
            litellm_metadata: object_field(fields, "litellm_metadata").cloned(),
            litellm_session_id: None,
            litellm_trace_id: None,
            prompt_id: None,
            prompt_variables: None,
            langfuse_masking_function: None,
        };
        let headers = object_field(fields, "proxy_server_request")
            .and_then(|request| request.get("headers"))
            .and_then(as_object)
            .map(proxy_headers);
        let settings = TagSettings {
            disable_user_agent_tags: self.settings.tags.disable_user_agent_tags,
            extra_spend_tag_headers: &self.settings.tags.extra_spend_tag_headers,
        };
        Ok(get_request_tags(&params, headers.as_ref(), &settings)?)
    }
}

fn object_field<'fields>(
    fields: Option<&'fields IndexMap<String, Json>>,
    key: &str,
) -> Option<&'fields IndexMap<String, Json>> {
    // a missing key and a value that is not an object both read as absent, the way litellm's kwargs chain does
    fields?.get(key).and_then(as_object)
}

fn push_value(entries: &mut Vec<(String, LabelDumpValue)>, name: &str, value: Option<String>) {
    if let Some(text) = value {
        entries.push((name.to_owned(), LabelDumpValue::String(text)));
    }
}

fn model_info_id(fields: Option<&IndexMap<String, Json>>) -> Option<String> {
    object_field(fields, "model_info")?
        .get("id")
        .and_then(as_text)
        .map(str::to_owned)
}

// litellm reads metadata.model_info.id, then request_data.model_info.id, and an empty first id falls through
fn proxy_model_id(
    metadata: Option<&IndexMap<String, Json>>,
    fields: Option<&IndexMap<String, Json>>,
) -> Option<String> {
    match model_info_id(metadata) {
        Some(id) if !id.is_empty() => Some(id),
        Some(_) | None => model_info_id(fields),
    }
}

fn proxy_headers(fields: &IndexMap<String, Json>) -> ProxyHeaders {
    let mut headers = ProxyHeaders::new();
    for (name, value) in fields {
        if let Json::Text(text) = value {
            headers.insert(name.clone(), text.clone());
        }
    }
    headers
}

fn proxy_stream_text(fields: Option<&IndexMap<String, Json>>) -> String {
    match fields.and_then(|request_fields| request_fields.get("stream")) {
        Some(Json::Bool(true)) => "True".to_owned(),
        Some(Json::Bool(false)) => "False".to_owned(),
        Some(Json::Null | Json::Number(_) | Json::Text(_) | Json::List(_) | Json::Object(_))
        | None => "None".to_owned(),
    }
}

mod tests;
