use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use sha2::{Digest as _, Sha256};
use std::str::from_utf8;
use url::form_urlencoded::Serializer;

use super::{
    Entropy, HTTP_TIMEOUT_SECONDS, Http, HttpBody,
    models::{CliAuthContract, PkceCredential, PkceFailure, RevocationUnavailable},
};

pub fn pkce_pair<E>(entropy: &mut E) -> Result<(String, String), PkceFailure>
where
    E: Entropy,
{
    let mut verifier_bytes = [0_u8; 64];
    entropy.fill(&mut verifier_bytes)?;
    let verifier = URL_SAFE_NO_PAD.encode(verifier_bytes);
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    Ok((verifier, challenge))
}

#[must_use]
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

pub fn register_client<H>(
    contract: &CliAuthContract,
    redirect_uri: &str,
    http: &H,
) -> Result<String, PkceFailure>
where
    H: Http,
{
    let registration = ClientRegistration {
        client_name: "litellm-cli",
        redirect_uris: vec![String::from(redirect_uri)],
        grant_types: ["authorization_code", "refresh_token"],
        response_types: ["code"],
        token_endpoint_auth_method: "none",
    };
    let body = serde_json::to_vec(&registration).map_err(|error| {
        PkceFailure::new_owned(format!("invalid registration request: {error}"))
    })?;
    let response = http.post(
        &contract.registration_endpoint,
        HttpBody::Json(body),
        HTTP_TIMEOUT_SECONDS,
        false,
    )?;
    refuse_redirect("client registration", &response)?;
    if response.status != 200 && response.status != 201 {
        return Err(PkceFailure::new_owned(format!(
            "client registration failed: {}",
            error_detail(&response)
        )));
    }
    let client_id =
        serde_json::from_slice::<RegisteredClient>(&response.body).map_err(|error| {
            PkceFailure::new_owned(format!("invalid registration response: {error}"))
        })?;
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

pub fn redeem_code<H>(
    contract: &CliAuthContract,
    request: &RedeemCodeRequest,
    http: &H,
    now_seconds: i64,
) -> Result<PkceCredential, PkceFailure>
where
    H: Http,
{
    validate_code_request(request)?;
    let code_has_payload = !request.code.is_empty();
    if !code_has_payload {
        return Err(PkceFailure::new("authorization code is empty"));
    }
    let client_has_payload = !request.client_id.is_empty();
    if !client_has_payload {
        return Err(PkceFailure::new("client ID is empty"));
    }
    authorization_exchange(contract, request, http, now_seconds)
}

fn validate_code_request(request: &RedeemCodeRequest) -> Result<(), PkceFailure> {
    if request.verifier.is_empty() {
        return Err(PkceFailure::new("code verifier is empty"));
    }
    let redirect_uri = request.redirect_uri.trim_end_matches('/');
    if redirect_uri.is_empty() {
        return Err(PkceFailure::new("redirect URI is empty"));
    }
    if !redirect_uri.starts_with("http://127.0.0.1:") {
        return Err(PkceFailure::new("redirect URI is not a loopback address"));
    }
    if request.code.chars().any(char::is_whitespace) {
        return Err(PkceFailure::new("authorization code contains whitespace"));
    }
    if request.code.len() > 8_192 || request.verifier.len() > 256 {
        return Err(PkceFailure::new("authorization code request is too large"));
    }
    if !redirect_uri.ends_with("/callback") {
        return Err(PkceFailure::new(
            "redirect URI does not use the callback path",
        ));
    }
    let code_prefix = request.code.as_bytes().first().copied();
    if code_prefix == Some(0) {
        return Err(PkceFailure::new("authorization code starts with NUL"));
    }
    let verifier_is_ascii = request.verifier.is_ascii();
    if !verifier_is_ascii {
        return Err(PkceFailure::new("code verifier is not ASCII"));
    }
    Ok(())
}

pub struct RefreshCredentialRequest {
    pub client_id: String,
    pub refresh_token: String,
}

pub fn refresh_credential<H>(
    contract: &CliAuthContract,
    request: &RefreshCredentialRequest,
    http: &H,
    now_seconds: i64,
) -> Result<PkceCredential, PkceFailure>
where
    H: Http,
{
    if request.refresh_token.is_empty() {
        return Err(PkceFailure::new("refresh token is empty"));
    }
    let token_length = request.refresh_token.len();
    if token_length > 0x4000 {
        return Err(PkceFailure::new("refresh token is too large"));
    }
    if request.refresh_token.chars().any(char::is_whitespace) {
        return Err(PkceFailure::new("refresh token contains whitespace"));
    }
    if request.client_id.is_empty() {
        return Err(PkceFailure::new("client ID is empty"));
    }
    if request.client_id.len() > 1_024 {
        return Err(PkceFailure::new("client ID is too large"));
    }
    if request.refresh_token.as_bytes().contains(&0) {
        return Err(PkceFailure::new("refresh token contains a NUL byte"));
    }
    let refresh_has_separator = request.refresh_token.contains(':');
    if refresh_has_separator && request.refresh_token.starts_with(':') {
        return Err(PkceFailure::new("refresh token has an invalid separator"));
    }
    let client_is_ascii = request.client_id.is_ascii();
    if !client_is_ascii {
        return Err(PkceFailure::new("client ID is not ASCII"));
    }
    let input = TokenInput::Refresh(request);
    token_request(contract, &request.client_id, http, now_seconds, input)
}

pub fn revoke_credential<H>(
    contract: &CliAuthContract,
    client_id: &str,
    refresh_token: &str,
    http: &H,
) -> Result<(), RevocationUnavailableOrFailure>
where
    H: Http,
{
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
        return Err(RevocationUnavailableOrFailure::Failure(
            PkceFailure::new_owned(error_detail(&response)),
        ));
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

#[derive(serde::Serialize)]
struct ClientRegistration {
    client_name: &'static str,
    redirect_uris: Vec<String>,
    grant_types: [&'static str; 2],
    response_types: [&'static str; 1],
    token_endpoint_auth_method: &'static str,
}

#[derive(serde::Deserialize)]
struct TokenResponse {
    access_token: String,
    expires_in: i64,
    refresh_token: String,
    user_id: Option<String>,
    team_id: Option<String>,
}

#[derive(Clone, Copy)]
enum TokenInput<'request> {
    Code(&'request RedeemCodeRequest),
    Refresh(&'request RefreshCredentialRequest),
}

fn token_request<H>(
    contract: &CliAuthContract,
    client_id: &str,
    http: &H,
    now_seconds: i64,
    input: TokenInput<'_>,
) -> Result<PkceCredential, PkceFailure>
where
    H: Http,
{
    let fields = token_fields(contract, input);
    let response = http.post(
        &contract.token_endpoint,
        HttpBody::Form(fields),
        HTTP_TIMEOUT_SECONDS,
        false,
    )?;
    refuse_redirect("token request", &response)?;
    if response.status != 200 {
        return Err(PkceFailure::new_owned(error_detail(&response)));
    }
    let token = parse_token(&response)?;
    let expires_at = now_seconds
        .checked_add(token.expires_in)
        .ok_or_else(|| PkceFailure::new("token expiry overflow"))?;
    Ok(PkceCredential {
        access_token: token.access_token,
        refresh_token: token.refresh_token,
        expires_at,
        client_id: client_id.to_owned(),
        token_endpoint: contract.token_endpoint.clone(),
        revocation_endpoint: contract.revocation_endpoint.clone(),
        resource: contract.resource.clone(),
        user_id: token.user_id,
        team_id: token.team_id,
    })
}

fn authorization_exchange<H>(
    contract: &CliAuthContract,
    request: &RedeemCodeRequest,
    http: &H,
    now_seconds: i64,
) -> Result<PkceCredential, PkceFailure>
where
    H: Http,
{
    let input = TokenInput::Code(request);
    token_request(contract, &request.client_id, http, now_seconds, input)
}

fn token_fields(contract: &CliAuthContract, input: TokenInput<'_>) -> Vec<(String, String)> {
    match input {
        TokenInput::Code(request) => vec![
            (
                String::from("grant_type"),
                String::from("authorization_code"),
            ),
            (String::from("code"), request.code.clone()),
            (String::from("redirect_uri"), request.redirect_uri.clone()),
            (String::from("client_id"), request.client_id.clone()),
            (String::from("code_verifier"), request.verifier.clone()),
            (String::from("resource"), contract.resource.clone()),
        ],
        TokenInput::Refresh(request) => vec![
            (String::from("grant_type"), String::from("refresh_token")),
            (String::from("refresh_token"), request.refresh_token.clone()),
            (String::from("client_id"), request.client_id.clone()),
            (String::from("resource"), contract.resource.clone()),
        ],
    }
}

fn form(fields: Vec<(&str, &str)>) -> Vec<(String, String)> {
    if fields.is_empty() {
        return Vec::new();
    }
    fields
        .into_iter()
        .map(|(key, value)| (key.to_owned(), value.to_owned()))
        .collect()
}

fn refuse_redirect(name: &str, response: &super::HttpResponse) -> Result<(), PkceFailure> {
    if response.status == 307 || response.status == 308 {
        return Err(PkceFailure::new_owned(format!(
            "{name} redirected; refusing to follow it"
        )));
    }
    Ok(())
}

fn error_detail(response: &super::HttpResponse) -> String {
    let fallback = from_utf8(&response.body).unwrap_or("");
    let detail = serde_json::from_slice::<ErrorBody>(&response.body)
        .ok()
        .and_then(|body| body.error_description.or(body.error).or(body.detail))
        .unwrap_or_else(|| String::from(fallback));
    detail.chars().take(200).collect()
}

#[derive(serde::Deserialize)]
struct ErrorBody {
    error_description: Option<String>,
    error: Option<String>,
    detail: Option<String>,
}

fn parse_token(response: &super::HttpResponse) -> Result<TokenResponse, PkceFailure> {
    let token: TokenResponse = serde_json::from_slice(&response.body)
        .map_err(|error| PkceFailure::new_owned(format!("invalid token response: {error}")))?;
    if token.access_token.is_empty() || token.refresh_token.is_empty() || token.expires_in <= 0 {
        return Err(PkceFailure::new("invalid token response fields"));
    }
    Ok(token)
}
