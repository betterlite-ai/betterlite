use betterlite_models::catalog::BundledCatalog;
use betterlite_vertex::endpoint_target::error::EndpointTargetError;
use betterlite_vertex::endpoint_target::headers::set_headers;
use betterlite_vertex::endpoint_target::partner::{PartnerUrlInput, create_vertex_url};
use betterlite_vertex::endpoint_target::passthrough::{
    construct_target_url, get_vertex_location_from_url, get_vertex_model_id_from_url,
    get_vertex_project_id_from_url,
};
use betterlite_vertex::endpoint_target::region::select_region;
use betterlite_vertex::endpoint_target::target::{
    AuthHeader, CustomProxyInput, EndpointTarget, PartnerRequest, TokenUrlInput,
    check_custom_proxy, get_api_base, get_complete_vertex_url, get_token_and_url,
};
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::{Value, json};

use super::input::{
    ApiBaseCase, CompletePartnerCase, CreatePartnerCase, GraftOperation, HeaderCase, Input,
    ProxyCase, RegionCase, RouteOperation, TokenCase, VersionName,
};

pub(crate) fn run(input: &Input) -> Value {
    match input {
        Input::UnsupportedRegionWarning(operation) => json!({
            "regions": region_rows(&operation.regions),
            "api_bases": api_base_rows(&operation.api_bases),
        }),
        Input::LlamaOpenapi(operation) => json!({"urls": complete_rows(&operation.complete)}),
        Input::Ai21Stream(operation) => json!({
            "complete": complete_rows(&operation.complete),
            "create": create_rows(&operation.create),
        }),
        Input::HostAndVersionGrafts(operation) => graft_operation(operation),
        Input::PscRequiredFields(operation) | Input::QueryPreserved(operation) => {
            json!({"targets": proxy_rows(&operation.targets)})
        }
        Input::GeminiCustomKey(operation) => json!({
            "tokens": token_rows(&operation.tokens),
            "proxies": proxy_rows(&operation.proxies),
        }),
        Input::HeaderOverride(operation) => json!({"headers": header_rows(&operation.headers)}),
        Input::CachePassthroughPath(operation) => route_operation(operation),
    }
}

fn region_rows(cases: &[RegionCase]) -> Value {
    let catalog = BundledCatalog::default().get();
    let rows: Vec<Value> = cases
        .iter()
        .map(|case| {
            let selection = select_region(&catalog, &case.model, case.region.as_deref());
            json!({
                "model": case.model,
                "configured": case.region,
                "selected": selection.region,
                "warned": selection.unsupported.is_some(),
            })
        })
        .collect();
    Value::Array(rows)
}

fn api_base_rows(cases: &[ApiBaseCase]) -> Value {
    let rows: Vec<Value> = cases
        .iter()
        .map(|case| {
            get_api_base(case.api_base.as_deref(), case.location.as_deref()).map_or_else(
                |_error| json!({"error": true}),
                |base| json!({"base": base}),
            )
        })
        .collect();
    Value::Array(rows)
}

fn complete_rows(cases: &[CompletePartnerCase]) -> Value {
    let catalog = BundledCatalog::default().get();
    let rows: Vec<Value> = cases
        .iter()
        .map(|case| {
            let request = PartnerRequest {
                custom_api_base: case.api_base.as_deref(),
                location: case.location.as_deref(),
                vertex_project: case.project.as_deref(),
                project_id: case.project_id.as_deref().unwrap_or(""),
                partner: case.partner.partner(),
                stream: case.stream,
                model: &case.model,
            };
            get_complete_vertex_url(&catalog, &request)
                .map_or_else(|_error| json!({"error": true}), |url| json!({"url": url}))
        })
        .collect();
    Value::Array(rows)
}

fn create_rows(cases: &[CreatePartnerCase]) -> Value {
    let rows: Vec<Value> = cases
        .iter()
        .map(|case| {
            let input = PartnerUrlInput {
                location: &case.location,
                project: &case.project,
                partner: case.partner.partner(),
                stream: case.stream,
                model: &case.model,
                api_base: case.api_base.as_deref(),
            };
            create_vertex_url(&input)
                .map_or_else(|_error| json!({"error": true}), |url| json!({"url": url}))
        })
        .collect();
    Value::Array(rows)
}

fn graft_operation(operation: &GraftOperation) -> Value {
    json!({
        "proxies": proxy_rows(&operation.proxies),
        "tokens": token_rows(&operation.tokens),
    })
}

fn token_rows(cases: &[TokenCase]) -> Value {
    let catalog = BundledCatalog::default().get();
    let rows: Vec<Value> = cases
        .iter()
        .map(|case| {
            let token = case
                .auth
                .as_ref()
                .map(|value| SecretString::from(value.clone()));
            let key = case
                .gemini_api_key
                .as_ref()
                .map(|value| SecretString::from(value.clone()));
            let input = TokenUrlInput {
                provider: case.provider.provider(),
                model: &case.model,
                auth_token: token.as_ref(),
                gemini_api_key: key.as_ref(),
                project: case.project.as_deref(),
                location: case.location.as_deref(),
                stream: case.stream,
                api_base: case.api_base.as_deref(),
                use_v1beta1_features: case.use_v1beta1_features,
                mode: case.mode.mode(),
                use_psc_endpoint_format: case.use_psc_endpoint_format,
            };
            target_json(get_token_and_url(&catalog, &input))
        })
        .collect();
    Value::Array(rows)
}

fn proxy_rows(cases: &[ProxyCase]) -> Value {
    let rows: Vec<Value> = cases
        .iter()
        .map(|case| {
            let auth = case
                .auth
                .as_ref()
                .map(|value| AuthHeader::Token(SecretString::from(value.clone())));
            let key = case
                .gemini_api_key
                .as_ref()
                .map(|value| SecretString::from(value.clone()));
            let input = CustomProxyInput {
                api_base: case.api_base.as_deref(),
                provider: case.provider.provider(),
                gemini_api_key: key.as_ref(),
                endpoint: case.endpoint.endpoint().as_str(),
                stream: case.stream,
                auth_header: auth.as_ref(),
                url: &case.url,
                model: case.model.as_deref(),
                project: case.project.as_deref(),
                location: case.location.as_deref(),
                api_version: case.version.map(VersionName::version),
                use_psc_endpoint_format: case.use_psc_endpoint_format,
            };
            target_json(check_custom_proxy(&input))
        })
        .collect();
    Value::Array(rows)
}

fn header_rows(cases: &[HeaderCase]) -> Value {
    let rows: Vec<Value> = cases
        .iter()
        .map(|case| {
            let token = case
                .auth
                .as_ref()
                .map(|value| SecretString::from(value.clone()));
            let headers = set_headers(token.as_ref(), &case.extras);
            let exposed: serde_json::Map<String, Value> = headers
                .iter()
                .map(|(name, value)| {
                    (
                        name.clone(),
                        Value::String(value.expose_secret().to_owned()),
                    )
                })
                .collect();
            Value::Object(exposed)
        })
        .collect();
    Value::Array(rows)
}

fn route_operation(operation: &RouteOperation) -> Value {
    let urls: Vec<Value> = operation
        .urls
        .iter()
        .map(|url| {
            json!({
                "url": url,
                "project": get_vertex_project_id_from_url(url),
                "location": get_vertex_location_from_url(url),
                "model": get_vertex_model_id_from_url(url),
            })
        })
        .collect();
    let targets: Vec<Value> = operation
        .routes
        .iter()
        .map(|case| {
            construct_target_url(
                &case.base_url,
                &case.route,
                case.project.as_deref(),
                case.location.as_deref(),
            )
            .map_or_else(|_error| json!({"error": true}), |url| json!({"url": url}))
        })
        .collect();
    json!({"urls": urls, "targets": targets})
}

fn target_json(result: Result<EndpointTarget, EndpointTargetError>) -> Value {
    match result {
        Ok(target) => json!({"url": target.url, "auth": auth_json(&target)}),
        Err(_error) => json!({"error": true}),
    }
}

fn auth_json(target: &EndpointTarget) -> Value {
    match &target.auth {
        None => Value::Null,
        Some(AuthHeader::Token(token)) => Value::String(token.expose_secret().to_owned()),
        Some(AuthHeader::Headers(headers)) => Value::Object(
            headers
                .iter()
                .map(|(name, value)| {
                    (
                        name.clone(),
                        Value::String(value.expose_secret().to_owned()),
                    )
                })
                .collect(),
        ),
    }
}
