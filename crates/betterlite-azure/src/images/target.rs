// LiteLLM's Azure image URL order: MAI first, then FLUX 2, then the v1 surface, then the dated deployment route
use crate::azure_urls::{AzureUrlError, get_azure_v1_image_url};

const MAI_DEFAULT_API_VERSION: &str = "preview";
const DEPLOYMENTS: &str = "/openai/deployments/";
const FLUX2_PROVIDERS: &str = "/providers/";
const GENERATION_ROUTE: &str = "/openai/images/generations";

#[derive(Debug, Clone, Copy, Default)]
pub struct AzureClientParams<'data> {
    pub azure_endpoint: Option<&'data str>,
    pub base_url: Option<&'data str>,
    pub api_version: Option<&'data str>,
}

// Deployment-scoped endpoints reach base_url, so the endpoint wins whenever the client kept it
pub fn create_azure_base_url(
    params: &AzureClientParams<'_>,
    model: Option<&str>,
    base_model: Option<&str>,
) -> Result<String, AzureUrlError> {
    let base = params
        .azure_endpoint
        .or(params.base_url)
        .unwrap_or_default()
        .trim_end_matches('/');
    let model = model.unwrap_or("");
    let mai_model = base_model.filter(|name| !name.is_empty()).unwrap_or(model);
    if is_mai_model(mai_model) {
        return Ok(mai_image_generation_url(base, params.api_version));
    }
    if is_flux2_model(model) {
        return Ok(flux2_image_generation_url(base, model, params.api_version));
    }
    if let Some(url) = get_azure_v1_image_url(base, params.api_version, GENERATION_ROUTE)? {
        return Ok(url);
    }
    Ok(dated_image_generation_url(
        base,
        model,
        params.api_version.unwrap_or_default(),
    ))
}

// Neither marker looks at case, and both ignore the separators a deployment name may carry
#[must_use]
pub fn is_mai_model(model: &str) -> bool {
    model
        .chars()
        .filter(|character| !matches!(character, '-' | '_'))
        .flat_map(char::to_lowercase)
        .collect::<String>()
        .contains("maiimage")
}

#[must_use]
pub fn is_flux2_model(model: &str) -> bool {
    let normalized = normalized_flux2(model);
    normalized.contains("flux-2") || normalized.contains("flux2")
}

fn normalized_flux2(model: &str) -> String {
    model.to_lowercase().replace(['.', '_'], "-")
}

// An api_base that already holds /mai/ is cut at its first /images/; otherwise /mai/v1/images/ is appended
fn mai_image_generation_url(base: &str, api_version: Option<&str>) -> String {
    let version = api_version
        .filter(|text| !text.is_empty())
        .unwrap_or(MAI_DEFAULT_API_VERSION);
    let (before_query, query) = base
        .split_once('?')
        .map_or((base, None), |(before, after)| (before, Some(after)));
    let trimmed = before_query.trim_end_matches('/');
    let path = if trimmed.contains("/mai/") {
        let prefix = trimmed
            .split_once("/images/")
            .map_or(trimmed, |(before, _)| before);
        format!("{prefix}/images/generations")
    } else {
        format!("{trimmed}/mai/v1/images/generations")
    };
    query.map_or_else(
        || format!("{path}?api-version={version}"),
        |original| format!("{path}?{original}"),
    )
}

fn flux2_image_generation_url(base: &str, model: &str, api_version: Option<&str>) -> String {
    let version = api_version
        .filter(|text| !text.is_empty())
        .unwrap_or(MAI_DEFAULT_API_VERSION);
    if base.contains(FLUX2_PROVIDERS) {
        return if base.contains('?') {
            base.to_owned()
        } else {
            format!("{base}?api-version={version}")
        };
    }
    let path = if normalized_flux2(model).contains("flux-2-flex") {
        "flux-2-flex"
    } else {
        "flux-2-pro"
    };
    format!("{base}/providers/blackforestlabs/v1/{path}?api-version={version}")
}

// The dated route is plain concatenation: an existing query on the base is kept as part of the base text
fn dated_image_generation_url(base: &str, model: &str, api_version: &str) -> String {
    let with_deployment = if base.contains(DEPLOYMENTS) {
        base.to_owned()
    } else {
        format!("{base}/openai/deployments/{model}")
    };
    format!("{with_deployment}/images/generations?api-version={api_version}")
}
