use url::Url;

use super::{
    DISCOVERY_PATH, HTTP_TIMEOUT_SECONDS, Http,
    models::{CliAuthContract, PkceFailure},
};

pub fn discover_cli_auth<H>(base_url: &str, http: &H) -> Result<CliAuthContract, PkceFailure>
where
    H: Http,
{
    let base = Url::parse(base_url)
        .map_err(|error| PkceFailure::new_owned(format!("invalid base URL: {error}")))?;
    let url = format!("{}{}", base.as_str().trim_end_matches('/'), DISCOVERY_PATH);
    let response = http.get(&url, HTTP_TIMEOUT_SECONDS)?;
    if response.status != 200 {
        return Err(PkceFailure::new_owned(format!(
            "discovery returned status {}",
            response.status
        )));
    }
    let contract: CliAuthContract = serde_json::from_slice(&response.body)
        .map_err(|error| PkceFailure::new_owned(format!("invalid discovery document: {error}")))?;
    validate_contract(&contract, base_url)?;
    Ok(contract)
}

fn validate_contract(contract: &CliAuthContract, base_url: &str) -> Result<(), PkceFailure> {
    if contract.contract_version != 1 {
        return Err(PkceFailure::new("unsupported discovery contract version"));
    }
    if !contract
        .supported_methods
        .iter()
        .any(|method| method == "S256")
    {
        return Err(PkceFailure::new("the proxy does not support PKCE S256"));
    }
    if canonical_url(&contract.issuer) != canonical_url(base_url) {
        return Err(PkceFailure::new(
            "discovery issuer does not match the base URL",
        ));
    }
    let expected_origin = origin(base_url);
    for endpoint in [
        &contract.authorization_endpoint,
        &contract.token_endpoint,
        &contract.registration_endpoint,
        &contract.revocation_endpoint,
        &contract.resource,
    ] {
        if origin(endpoint).as_deref() != expected_origin.as_deref() {
            return Err(PkceFailure::new(
                "discovery names an endpoint outside the proxy origin",
            ));
        }
    }
    Ok(())
}

#[must_use]
pub fn origin(value: &str) -> Option<String> {
    let parsed = Url::parse(value).ok()?;
    let scheme = parsed.scheme();
    if scheme != "http" && scheme != "https" {
        return None;
    }
    let host = parsed.host_str()?;
    let port = parsed.port_or_known_default()?;
    Some(format!("{scheme}://{host}:{port}"))
}

fn canonical_url(value: &str) -> Option<String> {
    let parsed = Url::parse(value).ok()?;
    let normalized_origin = origin(value)?;
    let path = parsed.path().trim_end_matches('/');
    Some(format!("{normalized_origin}{path}"))
}
