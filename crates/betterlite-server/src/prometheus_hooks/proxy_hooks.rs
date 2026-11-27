use std::future::{Future, ready};
use std::time::Instant;

use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use betterlite_core::agents::json::AgentJson;
use betterlite_core::api::request_principal_metadata::PrincipalFacts;
use betterlite_core::api::request_principal_metadata::principal::logged_api_key;
use betterlite_logging::logging_metadata::params::LitellmParams;
use betterlite_logging::logging_metadata::tags::{ProxyHeaders, TagSettings, get_request_tags};
use betterlite_logging::prometheus::failure::RequestedModelCatalog;
use betterlite_logging::prometheus::failure::exception::LabelException;
use betterlite_logging::prometheus::failure::exception_name::exception_class_name;
use betterlite_logging::prometheus::failure::model_cardinality::bounded_requested_model_label;
use betterlite_logging::prometheus::failure::provider::{
    ProviderRequestData, api_provider_from_exception,
};
use betterlite_logging::prometheus::failure::rate_limit::extract_rate_limit_labels;
use betterlite_logging::prometheus::failure::status_code::extract_status_code;
use betterlite_logging::prometheus_label_projection::label_factory_context::PrometheusLabelFactoryContext;
use betterlite_logging::prometheus_label_projection::prometheus_label_factory::LabelSource;
use betterlite_models::decisions::Json;
use betterlite_openai::openai_base_config::Object as JsonMap;
use betterlite_prometheus::label_value_record::UserApiKeyLabelValues;
use indexmap::IndexMap;

use super::error::PrometheusHookError;
use super::hooks::PrometheusHooks;
use super::json::{as_object, as_text};
use super::labels::text_tags;

impl PrometheusHooks {
    // the total requests counter is counted in the success event, so the proxy success hook stays empty
    #[must_use]
    pub fn async_post_call_success_hook() -> impl Future<Output = ()> + Send {
        ready(())
    }

    // ast-grep-ignore: no-long-names
    #[must_use]
    pub fn async_post_call_failure_hook<'hook>(
        &'hook self,
        request_data: &'hook JsonMap,
        exception: &'hook LabelException,
        auth: &'hook UserAPIKeyAuth,
    ) -> impl Future<Output = ()> + Send + 'hook {
        if let Err(error) = self.record_proxy_failure(request_data, exception, auth) {
            tracing::error!("prometheus Layer Error(): Exception occured - {error}");
        }
        ready(())
    }

    fn record_proxy_failure(
        &self,
        request_data: &JsonMap,
        exception: &LabelException,
        auth: &UserAPIKeyAuth,
    ) -> Result<(), PrometheusHookError> {
        let status_code = extract_status_code(None, Some(exception), None);
        let data = request_json(request_data);
        let fields = as_object(&data);
        let tags = self.proxy_tags(fields)?;
        let metadata = fields
            .and_then(|fields| fields.get("metadata"))
            .and_then(as_object);
        let model_id = proxy_model_id(metadata, fields);
        let (rate_limit_category, rate_limit_type) = extract_rate_limit_labels(Some(exception));
        let api_provider = self.proxy_api_provider(fields, exception);
        let principal: PrincipalFacts = serde_json::to_value(auth)
            .and_then(serde_json::from_value)
            .map_err(PrometheusHookError::Principal)?;
        let hashed_api_key = match status_code {
            Some(401) => None,
            Some(_) | None => logged_api_key(&principal).map(str::to_owned),
        };
        let status_text = status_code.map_or_else(|| "None".to_owned(), |code| code.to_string());
        let values = UserApiKeyLabelValues {
            end_user: principal.end_user_id.clone(),
            user: principal.user_id.clone(),
            user_email: principal.user_email.clone(),
            hashed_api_key,
            api_key_alias: principal.key_alias.clone(),
            team: principal.team_id.clone(),
            team_alias: principal.team_alias.clone(),
            org_id: principal.org_id.clone(),
            org_alias: principal.organization_alias.clone(),
            requested_model: self.proxy_requested_model(fields),
            status_code: Some(status_text.clone()),
            exception_status: Some(status_text),
            exception_class: Some(exception_class_name(exception)),
            rate_limit_category,
            rate_limit_type,
            tags: text_tags(&tags),
            route: principal.request_route.clone(),
            client_ip: metadata_text(metadata, "requester_ip_address"),
            user_agent: metadata_text(metadata, "user_agent"),
            model_id,
            api_provider,
            stream: self
                .settings
                .labels
                .emit_stream_label
                .then(|| proxy_stream_text(fields)),
            ..UserApiKeyLabelValues::default()
        };
        let context = PrometheusLabelFactoryContext::new(
            &values,
            &self.settings.labels,
            self.settings.end_user,
        )?;
        let metrics = self.metrics();
        let now = Instant::now();
        for (metric, name) in [
            (
                &self.collectors.litellm_proxy_failed_requests_metric,
                "litellm_proxy_failed_requests_metric",
            ),
            (
                &self.collectors.litellm_proxy_total_requests_metric,
                "litellm_proxy_total_requests_metric",
            ),
        ] {
            metrics.emit_one(metric, name, &values, LabelSource::Context(&context), now)?;
        }
        Ok(())
    }

    fn proxy_requested_model(&self, fields: Option<&IndexMap<String, Json>>) -> Option<String> {
        let requested = fields
            .and_then(|fields| fields.get("model"))
            .cloned()
            .unwrap_or_else(|| Json::Text(String::new()));
        let catalog = self.catalog();
        bounded_requested_model_label(
            Some(&requested),
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
            .and_then(|fields| fields.get("litellm_params"))
            .and_then(as_object);
        let standard_logging = fields
            .and_then(|fields| fields.get("standard_logging_object"))
            .and_then(as_object);
        let model = litellm_params
            .and_then(|params| params.get("model"))
            .and_then(as_text)
            .filter(|model| !model.is_empty())
            .or_else(|| {
                fields
                    .and_then(|fields| fields.get("model"))
                    .and_then(as_text)
                    .filter(|model| !model.is_empty())
            });
        let request = ProviderRequestData {
            litellm_params,
            standard_logging,
            model,
        };
        self.provider
            .api_provider(&request)
            .or_else(|| api_provider_from_exception(exception))
    }

    fn proxy_tags(
        &self,
        fields: Option<&IndexMap<String, Json>>,
    ) -> Result<Vec<Json>, PrometheusHookError> {
        let params = LitellmParams {
            metadata: object_field(fields, "metadata").cloned(),
            litellm_metadata: object_field(fields, "litellm_metadata").cloned(),
            ..LitellmParams::default()
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

fn request_json(request_data: &JsonMap) -> Json {
    serde_json::from_value(AgentJson::Object(request_data.clone()))
        .unwrap_or_else(|_error| Json::Object(IndexMap::new()))
}

fn object_field<'fields>(
    fields: Option<&'fields IndexMap<String, Json>>,
    key: &str,
) -> Option<&'fields IndexMap<String, Json>> {
    fields?.get(key).and_then(as_object)
}

fn metadata_text(metadata: Option<&IndexMap<String, Json>>, key: &str) -> Option<String> {
    metadata?.get(key).and_then(as_text).map(str::to_owned)
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
    match fields.and_then(|fields| fields.get("stream")) {
        Some(Json::Bool(true)) => "True".to_owned(),
        Some(Json::Bool(false)) => "False".to_owned(),
        Some(Json::Null | Json::Number(_) | Json::Text(_) | Json::List(_) | Json::Object(_))
        | None => "None".to_owned(),
    }
}

#[cfg(test)]
mod tests;
