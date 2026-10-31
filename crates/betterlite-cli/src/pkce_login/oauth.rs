use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use secrecy::SecretString;
use sha2::{Digest as _, Sha256};
use url::form_urlencoded::Serializer;

use super::{
    Entropy, HTTP_TIMEOUT_SECONDS, Http, HttpBody, WallClock,
    models::{CliAuthContract, PkceCredential, PkceFailure},
    wire::{
        ClientRegistration, RegisteredClient, error_detail, parse_and_require, parse_token,
        refuse_redirect,
    },
};

pub fn pkce_pair<E>(entropy: &mut E) -> Result<(SecretString, String), PkceFailure>
where
    E: Entropy,
{
    let mut verifier_bytes = [0_u8; 64];
    entropy.fill(&mut verifier_bytes)?;
    let verifier = URL_SAFE_NO_PAD.encode(verifier_bytes);
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    Ok((SecretString::from(verifier), challenge))
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
    let body = serde_json::to_vec(&registration)
        .map_err(|source| PkceFailure::RegistrationRequest { source })?;
    let response = http.post(
        &contract.registration_endpoint,
        HttpBody::Json(body),
        HTTP_TIMEOUT_SECONDS,
        false,
    )?;
    refuse_redirect("client registration", &response)?;
    if response.status != 200 && response.status != 201 {
        return Err(PkceFailure::Protocol(format!(
            "client registration failed: {}",
            error_detail(&response.body)
        )));
    }
    let registered: RegisteredClient = parse_and_require(
        &response.body,
        |source| PkceFailure::RegistrationResponse { source },
        |registered: &RegisteredClient| registered.client_id.is_empty(),
        "registration returned an empty client_id",
    )?;
    Ok(registered.client_id)
}

pub struct RedeemCodeRequest {
    pub client_id: String,
    pub redirect_uri: String,
    pub code: SecretString,
    pub verifier: SecretString,
}

pub struct RefreshCredentialRequest {
    pub client_id: String,
    pub refresh_token: SecretString,
}

// A macro, not two near-identical functions, carries the one difference between the grants: their form fields.
macro_rules! token_grant_fn {
    ($name:ident, $request_ty:ty, |$request:ident, $contract:ident| $form:expr) => {
        pub fn $name<H, W>(
            $contract: &CliAuthContract,
            $request: &$request_ty,
            http: &H,
            wall_clock: &W,
        ) -> Result<PkceCredential, PkceFailure>
        where
            H: Http,
            W: WallClock,
        {
            let form = $form;
            token_request($contract, &$request.client_id, http, wall_clock, form)
        }
    };
}

token_grant_fn!(redeem_code, RedeemCodeRequest, |request, contract| vec![
    (
        String::from("grant_type"),
        SecretString::from("authorization_code"),
    ),
    (String::from("code"), request.code.clone()),
    (
        String::from("redirect_uri"),
        SecretString::from(request.redirect_uri.clone()),
    ),
    (
        String::from("client_id"),
        SecretString::from(request.client_id.clone()),
    ),
    (String::from("code_verifier"), request.verifier.clone()),
    (
        String::from("resource"),
        SecretString::from(contract.resource.clone()),
    ),
]);

token_grant_fn!(
    refresh_credential,
    RefreshCredentialRequest,
    |request, contract| vec![
        (
            String::from("grant_type"),
            SecretString::from("refresh_token"),
        ),
        (String::from("refresh_token"), request.refresh_token.clone()),
        (
            String::from("client_id"),
            SecretString::from(request.client_id.clone()),
        ),
        (
            String::from("resource"),
            SecretString::from(contract.resource.clone()),
        ),
    ]
);

fn token_request<H, W>(
    contract: &CliAuthContract,
    client_id: &str,
    http: &H,
    wall_clock: &W,
    form: Vec<(String, SecretString)>,
) -> Result<PkceCredential, PkceFailure>
where
    H: Http,
    W: WallClock,
{
    let response = http.post(
        &contract.token_endpoint,
        HttpBody::Form(form),
        HTTP_TIMEOUT_SECONDS,
        false,
    )?;
    refuse_redirect("token request", &response)?;
    if response.status != 200 {
        return Err(PkceFailure::Protocol(error_detail(&response.body)));
    }
    let token = parse_token(&response.body)?;
    let expires_at = wall_clock
        .now_seconds()
        .checked_add(token.expires_in)
        .ok_or_else(|| PkceFailure::Protocol("token expiry overflow".to_owned()))?;
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
