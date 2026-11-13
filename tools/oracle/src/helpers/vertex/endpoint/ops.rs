use betterlite_models::catalog::{BundledCatalog, ModelCatalog};
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
use betterlite_vertex::model_urls::ApiVersion;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::{Value, json};

use super::input::{
    ApiBaseCase, CompletePartnerCase, CreatePartnerCase, HeaderCase, Input, ProxyCase, RegionCase,
    RouteCase, RouteOperation, TokenCase, VersionName,
};

pub(crate) fn run(input: &Input) -> Value {
    let catalog = BundledCatalog::default().get();
    match input {
        Input::UnsupportedRegionWarning(operation) => json!({
            "regions": rows(&operation.regions, |case| region_row(&catalog, case)),
            "api_bases": rows(&operation.api_bases, api_base_row),
        }),
        Input::LlamaOpenapi(operation) => json!({
            "urls": rows(&operation.complete, |case| complete_row(&catalog, case)),
        }),
        Input::Ai21Stream(operation) => json!({
            "complete": rows(&operation.complete, |case| complete_row(&catalog, case)),
            "create": rows(&operation.create, create_row),
        }),
        Input::HostAndVersionGrafts(operation) => json!({
            "proxies": rows(&operation.proxies, proxy_row),
            "tokens": rows(&operation.tokens, |case| token_row(&catalog, case)),
        }),
        Input::PscRequiredFields(operation) | Input::QueryPreserved(operation) => {
            json!({"targets": rows(&operation.targets, proxy_row)})
        }
        Input::GeminiCustomKey(operation) => json!({
            "tokens": rows(&operation.tokens, |case| token_row(&catalog, case)),
            "proxies": rows(&operation.proxies, proxy_row),
        }),
        Input::HeaderOverride(operation) => {
            json!({"headers": rows(&operation.headers, header_row)})
        }
        Input::CachePassthroughPath(operation) => route_operation(operation),
    }
}

fn rows<Case, Row>(cases: &[Case], row: Row) -> Value
where
    Row: Fn(&Case) -> Value,
{
    let mut values = Vec::with_capacity(cases.len());
    for case in cases {
        values.push(row(case));
    }
    Value::Array(values)
}

fn region_row(catalog: &ModelCatalog, case: &RegionCase) -> Value {
    let selection = select_region(catalog, &case.model, case.region.as_deref());
    json!({
        "model": case.model,
        "configured": case.region,
        "selected": selection.region,
        "warned": selection.unsupported.is_some(),
    })
}

fn api_base_row(case: &ApiBaseCase) -> Value {
    match get_api_base(case.api_base.as_deref(), case.location.as_deref()) {
        Ok(base) => json!({"base": base}),
        Err(_error) => json!({"error": true}),
    }
}

fn complete_row(catalog: &ModelCatalog, case: &CompletePartnerCase) -> Value {
    let request = PartnerRequest {
        custom_api_base: case.api_base.as_deref(),
        location: case.location.as_deref(),
        vertex_project: case.project.as_deref(),
        project_id: case.project_id.as_deref().unwrap_or(""),
        partner: case.partner.partner(),
        stream: case.stream.unwrap_or(false),
        model: &case.model,
    };
    match get_complete_vertex_url(catalog, &request) {
        Ok(url) => json!({"url": url}),
        Err(_error) => json!({"error": true}),
    }
}

fn create_row(case: &CreatePartnerCase) -> Value {
    let input = PartnerUrlInput {
        location: &case.location,
        project: &case.project,
        partner: case.partner.partner(),
        stream: case.stream.unwrap_or(false),
        model: &case.model,
        api_base: case.api_base.as_deref(),
    };
    url_json(create_vertex_url(&input))
}

fn token_row(catalog: &ModelCatalog, case: &TokenCase) -> Value {
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
        stream: case.stream.unwrap_or(false),
        api_base: case.api_base.as_deref(),
        api_version: if case.use_v1beta1_features {
            ApiVersion::V1Beta1
        } else {
            ApiVersion::V1
        },
        mode: case.mode.mode(),
        use_psc_endpoint_format: case.use_psc_endpoint_format,
    };
    target_json(get_token_and_url(catalog, &input))
}

fn proxy_row(case: &ProxyCase) -> Value {
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
        stream: case.stream.unwrap_or(false),
        auth_header: auth.as_ref(),
        url: &case.url,
        model: case.model.as_deref(),
        project: case.project.as_deref(),
        location: case.location.as_deref(),
        api_version: case.version.map_or(ApiVersion::V1, VersionName::version),
        use_psc_endpoint_format: case.use_psc_endpoint_format,
    };
    target_json(check_custom_proxy(&input))
}

fn header_row(case: &HeaderCase) -> Value {
    let token = case.auth.as_deref().map(SecretString::from);
    let mut exposed = serde_json::Map::new();
    for (name, value) in set_headers(token.as_ref(), &case.extras) {
        exposed.insert(name, Value::String(value.expose_secret().to_owned()));
    }
    Value::Object(exposed)
}

fn route_operation(operation: &RouteOperation) -> Value {
    json!({
        "urls": rows(&operation.urls, extraction_row),
        "targets": rows(&operation.routes, target_row),
    })
}

fn extraction_row(url: &String) -> Value {
    json!({
        "url": url,
        "project": get_vertex_project_id_from_url(url),
        "location": get_vertex_location_from_url(url),
        "model": get_vertex_model_id_from_url(url),
    })
}

fn target_row(case: &RouteCase) -> Value {
    construct_target_url(
        &case.base_url,
        &case.route,
        case.project.as_deref(),
        case.location.as_deref(),
    )
    .map_or_else(|_error| json!({"error": true}), |url| json!({"url": url}))
}

fn url_json(result: Result<String, EndpointTargetError>) -> Value {
    match result {
        Ok(url) => json!({"url": url}),
        Err(_error) => json!({"error": true}),
    }
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
