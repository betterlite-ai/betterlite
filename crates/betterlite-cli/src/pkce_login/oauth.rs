use base64::Engine as _;
use sha2::{Digest as _, Sha256};
use url::form_urlencoded::Serializer;

use super::{
    CliAuthContract, Entropy, HTTP_TIMEOUT_SECONDS, Http, HttpBody, PkceCredential, PkceFailure,
    RevocationUnavailable,
};

pub fn pkce_pair(entropy: &mut impl Entropy) -> Result<(String, String), PkceFailure> {
    let mut verifier_bytes = [0_u8; 64];
    entropy.fill(&mut verifier_bytes)?;
    let verifier = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(verifier_bytes);
    let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(Sha256::digest(verifier.as_bytes()));
    Ok((verifier, challenge))
}

pub fn authorize_url(
    contract: &CliAuthContract,
    client_id: &str,
    redirect_uri: &str,
    state: &str,
    challenge: &str,
) -> String {
    let mut query = Serializer::new(String::new());
    query.append_pair("response_type", "code");
    query.append_pair("client_id", client_id);
    query.append_pair("redirect_uri", redirect_uri);
    query.append_pair("state", state);
    query.append_pair("code_challenge", challenge);
    query.append_pair("code_challenge_method", "S256");
    query.append_pair("resource", &contract.resource);
    format!("{}?{}", contract.authorization_endpoint, query.finish())
}

pub fn register_client<H: Http>(
    contract: &CliAuthContract,
    redirect_uri: &str,
    http: &H,
) -> Result<String, PkceFailure> {
    let body = serde_json::json!({
        "client_name": "litellm-cli",
        "redirect_uris": [redirect_uri],
        "grant_types": ["authorization_code", "refresh_token"],
        "response_types": ["code"],
        "token_endpoint_auth_method": "none"
    });
    let response = http.post(
        &contract.registration_endpoint,
        HttpBody::Json(body.to_string().into_bytes()),
        HTTP_TIMEOUT_SECONDS,
        false,
    )?;
    refuse_redirect("client registration", &response)?;
    if response.status != 200 && response.status != 201 {
        return Err(PkceFailure::new(format!(
            "client registration failed: {}",
            error_detail(&response)
        )));
    }
    let client_id = serde_json::from_slice::<RegisteredClient>(&response.body)
        .map_err(|error| PkceFailure::new(format!("invalid registration response: {error}")))?;
    if client_id.client_id.is_empty() {
        return Err(PkceFailure::new("registration returned an empty client_id"));
    }
    Ok(client_id.client_id)
}

pub struct RedeemCodeRequest {
    pub client_id: String,
    pub redirect_uri: String,
    pub code: String,
    pub verifier: String,
}

pub fn redeem_code<H: Http>(
    contract: &CliAuthContract,
    request: &RedeemCodeRequest,
    http: &H,
    now_seconds: i64,
) -> Result<PkceCredential, PkceFailure> {
    let fields = vec![
        ("grant_type", "authorization_code"),
        ("code", &request.code),
        ("redirect_uri", &request.redirect_uri),
        ("client_id", &request.client_id),
        ("code_verifier", &request.verifier),
        ("resource", &contract.resource),
    ];
    token_request(contract, &request.client_id, http, now_seconds, fields)
}

pub struct RefreshCredentialRequest {
    pub client_id: String,
    pub refresh_token: String,
}

pub fn refresh_credential<H: Http>(
    contract: &CliAuthContract,
    request: &RefreshCredentialRequest,
    http: &H,
    now_seconds: i64,
) -> Result<PkceCredential, PkceFailure> {
    let fields = vec![
        ("grant_type", "refresh_token"),
        ("refresh_token", &request.refresh_token),
        ("client_id", &request.client_id),
        ("resource", &contract.resource),
    ];
    token_request(contract, &request.client_id, http, now_seconds, fields)
}

pub fn revoke_credential<H: Http>(
    contract: &CliAuthContract,
    client_id: &str,
    refresh_token: &str,
    http: &H,
) -> Result<(), RevocationUnavailableOrFailure> {
    let response = http
        .post(
            &contract.revocation_endpoint,
            HttpBody::Form(form(vec![
                ("token", refresh_token),
                ("token_type_hint", "refresh_token"),
                ("client_id", client_id),
            ])),
            HTTP_TIMEOUT_SECONDS,
            false,
        )
        .map_err(RevocationUnavailableOrFailure::Failure)?;
    refuse_redirect("revocation", &response).map_err(RevocationUnavailableOrFailure::Failure)?;
    if response.status == 503 {
        return Err(RevocationUnavailableOrFailure::Unavailable(
            RevocationUnavailable {
                reason: error_detail(&response),
            },
        ));
    }
    if response.status != 200 {
        return Err(RevocationUnavailableOrFailure::Failure(PkceFailure::new(
            error_detail(&response),
        )));
    }
    Ok(())
}

#[derive(Debug, PartialEq, Eq)]
pub enum RevocationUnavailableOrFailure {
    Unavailable(RevocationUnavailable),
    Failure(PkceFailure),
}

#[derive(serde::Deserialize)]
struct RegisteredClient {
    client_id: String,
}

#[derive(serde::Deserialize)]
struct TokenResponse {
    access_token: String,
    expires_in: i64,
    refresh_token: String,
    user_id: Option<String>,
    team_id: Option<String>,
}

fn token_request<H: Http>(
    contract: &CliAuthContract,
    client_id: &str,
    http: &H,
    now_seconds: i64,
    fields: Vec<(&str, &str)>,
) -> Result<PkceCredential, PkceFailure> {
    let response = http.post(
        &contract.token_endpoint,
        HttpBody::Form(form(fields)),
        HTTP_TIMEOUT_SECONDS,
        false,
    )?;
    refuse_redirect("token request", &response)?;
    if response.status != 200 {
        return Err(PkceFailure::new(error_detail(&response)));
    }
    let token: TokenResponse = serde_json::from_slice(&response.body)
        .map_err(|error| PkceFailure::new(format!("invalid token response: {error}")))?;
    if token.access_token.is_empty() || token.refresh_token.is_empty() || token.expires_in <= 0 {
        return Err(PkceFailure::new("invalid token response fields"));
    }
    Ok(PkceCredential {
        access_token: token.access_token,
        refresh_token: token.refresh_token,
        expires_at: now_seconds
            .checked_add(token.expires_in)
            .ok_or_else(|| PkceFailure::new("token expiry overflow"))?,
        client_id: client_id.to_owned(),
        token_endpoint: contract.token_endpoint.clone(),
        revocation_endpoint: contract.revocation_endpoint.clone(),
        resource: contract.resource.clone(),
        user_id: token.user_id,
        team_id: token.team_id,
    })
}

fn form(fields: Vec<(&str, &str)>) -> Vec<(String, String)> {
    let pairs = fields
        .into_iter()
        .map(|(key, value)| (key.to_owned(), value.to_owned()))
        .collect();
    pairs
}

fn refuse_redirect(name: &str, response: &super::HttpResponse) -> Result<(), PkceFailure> {
    if response.status == 307 || response.status == 308 {
        return Err(PkceFailure::new(format!(
            "{name} redirected; refusing to follow it"
        )));
    }
    Ok(())
}

fn error_detail(response: &super::HttpResponse) -> String {
    let fallback = String::from_utf8_lossy(&response.body);
    let detail = serde_json::from_slice::<serde_json::Value>(&response.body)
        .ok()
        .and_then(|value| {
            value
                .get("error_description")
                .or_else(|| value.get("error"))
                .or_else(|| value.get("detail"))
                .and_then(serde_json::Value::as_str)
                .map(ToOwned::to_owned)
        })
        .unwrap_or_else(|| fallback.into_owned());
    detail.chars().take(200).collect()
}
