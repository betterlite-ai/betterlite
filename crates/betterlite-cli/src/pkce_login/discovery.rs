use url::Url;

use super::{
    DISCOVERY_PATH, HTTP_TIMEOUT_SECONDS, Http,
    models::{CliAuthContract, PkceFailure},
};

pub fn discover_cli_auth<H>(base_url: &str, http: &H) -> Result<CliAuthContract, PkceFailure>
where
    H: Http,
{
    let base = Url::parse(base_url).map_err(|source| PkceFailure::InvalidBaseUrl { source })?;
    let url = format!("{}{}", base.as_str().trim_end_matches('/'), DISCOVERY_PATH);
    let response = http.get(&url, HTTP_TIMEOUT_SECONDS)?;
    if response.status != 200 {
        return Err(PkceFailure::DiscoveryStatus {
            status: response.status,
        });
    }
    let contract: CliAuthContract = serde_json::from_slice(&response.body)
        .map_err(|source| PkceFailure::DiscoveryDocument { source })?;
    validate_contract(&contract, base_url)?;
    Ok(contract)
}

fn validate_contract(contract: &CliAuthContract, base_url: &str) -> Result<(), PkceFailure> {
    if contract.contract_version != 1 {
        return Err(PkceFailure::Protocol(
            "unsupported discovery contract version".to_owned(),
        ));
    }
    if !contract
        .supported_methods
        .iter()
        .any(|method| method == "S256")
    {
        return Err(PkceFailure::Protocol(
            "the proxy does not support PKCE S256".to_owned(),
        ));
    }
    if canonical_url(&contract.issuer) != canonical_url(base_url) {
        return Err(PkceFailure::Protocol(
            "discovery issuer does not match the base URL".to_owned(),
        ));
    }
    let base_origin = origin(base_url);
    let Some(expected_origin) = base_origin else {
        return Err(PkceFailure::Protocol(
            "discovery base URL has no HTTP origin".to_owned(),
        ));
    };
    for endpoint in [
        &contract.authorization_endpoint,
        &contract.token_endpoint,
        &contract.registration_endpoint,
        &contract.revocation_endpoint,
        &contract.resource,
    ] {
        if origin(endpoint).as_deref() != Some(expected_origin.as_str()) {
            return Err(PkceFailure::Protocol(
                "discovery names an endpoint outside the proxy origin".to_owned(),
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
