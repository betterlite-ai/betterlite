use std::time::{Duration, SystemTime};

use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use http::Method;
use http::header::{AUTHORIZATION, HeaderMap, HeaderValue};
use jiff::civil::DateTime;
use jiff::tz::TimeZone;
use secrecy::{ExposeSecret as _, SecretString};
use serde::{Deserialize, Serialize};
use serde_with::{DisplayFromStr, PickFirst, serde_as};
use url::Url;
use url::form_urlencoded::Serializer;

use super::{AwsCredentialsResolver, AwsFederation, FederationError};

const GRANT_TYPE: &str = "urn:ietf:params:oauth:grant-type:token-exchange";
const REQUESTED_TOKEN_TYPE: &str = "urn:ietf:params:oauth:token-type:access_token";
const IAM_SCOPE: &str = "https://www.googleapis.com/auth/iam";
pub(super) const TOKEN_URL_TEMPLATE: &str = "https://sts.{universe_domain}/v1/token";
const DEFAULT_UNIVERSE_DOMAIN: &str = "googleapis.com";
const IMPERSONATED_LIFETIME: &str = "3600s";
const EXPIRE_TIME_FORMAT: &str = "%Y-%m-%dT%H:%M:%SZ";
const IMPERSONATION_MARKER: &str = ":generateAccessToken";
const SUCCESS_STATUS: u16 = 200;
// google-auth's requests transport waits this long for each call
const REQUEST_DEADLINE: Duration = Duration::from_secs(120);

#[derive(Debug, Clone)]
pub struct AccessToken {
    pub token: SecretString,
    pub expires_at: SystemTime,
}

#[serde_as]
#[derive(Deserialize)]
struct StsResponse {
    access_token: SecretString,
    #[serde_as(as = "PickFirst<(_, DisplayFromStr)>")]
    expires_in: u64,
}

#[derive(Serialize)]
struct ImpersonationBody<'scopes> {
    delegates: Option<Vec<String>>,
    scope: Option<&'scopes [String]>,
    lifetime: &'static str,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ImpersonationResponse {
    access_token: SecretString,
    expire_time: String,
}

fn service_account_email(endpoint: &str) -> Option<&str> {
    let (before_marker, after_marker) = endpoint.split_once(IMPERSONATION_MARKER)?;
    if after_marker.contains('/') {
        return None;
    }
    let (_, account) = before_marker.rsplit_once('/')?;
    Some(account).filter(|name| !name.is_empty())
}

// google-auth checks the impersonation target before it asks AWS for anything
pub async fn access_token<R>(
    federation: &AwsFederation<'_, R>,
    client: &ProxiedClient,
    now: SystemTime,
) -> Result<AccessToken, FederationError>
where
    R: AwsCredentialsResolver + Sync,
{
    if let Some(endpoint) = &federation.impersonation_url
        && service_account_email(endpoint).is_none()
    {
        return Err(FederationError::ImpersonationTarget);
    }
    let subject_token = federation.subject_token(now).await?;
    let scope = if federation.impersonation_url.is_some() {
        IAM_SCOPE.to_owned()
    } else {
        federation.scopes.join(" ")
    };
    let exchanged = client
        .send_buffered(sts_request(federation, &subject_token, &scope)?)
        .await
        .map_err(FederationError::Transport)?;
    if exchanged.status != SUCCESS_STATUS {
        return Err(FederationError::StsStatus {
            status: exchanged.status,
        });
    }
    let sts: StsResponse =
        serde_json::from_slice(&exchanged.body).map_err(FederationError::StsResponse)?;
    let Some(endpoint) = &federation.impersonation_url else {
        let expires_at = now
            .checked_add(Duration::from_secs(sts.expires_in))
            .ok_or(FederationError::ExpiryOverflow)?;
        return Ok(AccessToken {
            token: sts.access_token,
            expires_at,
        });
    };
    impersonate(client, &federation.scopes, endpoint, &sts.access_token).await
}

async fn impersonate(
    client: &ProxiedClient,
    scopes: &[String],
    endpoint: &str,
    sts_token: &SecretString,
) -> Result<AccessToken, FederationError> {
    let request = impersonation_request(scopes, endpoint, sts_token)?;
    let response = client
        .send_buffered(request)
        .await
        .map_err(FederationError::Transport)?;
    if response.status != SUCCESS_STATUS {
        return Err(FederationError::ImpersonationStatus {
            status: response.status,
        });
    }
    let parsed: ImpersonationResponse =
        serde_json::from_slice(&response.body).map_err(FederationError::ImpersonationResponse)?;
    let expires = DateTime::strptime(EXPIRE_TIME_FORMAT, &parsed.expire_time)
        .and_then(|expire_time| expire_time.to_zoned(TimeZone::UTC))
        .map_err(FederationError::ExpireTime)?;
    Ok(AccessToken {
        token: parsed.access_token,
        expires_at: SystemTime::from(expires.timestamp()),
    })
}

pub(super) fn token_endpoint<R>(federation: &AwsFederation<'_, R>) -> Result<Url, FederationError> {
    let configured = federation
        .token_url
        .as_deref()
        .ok_or(FederationError::TokenUrlRequired)?;
    let universe = federation
        .universe_domain
        .as_deref()
        .unwrap_or(DEFAULT_UNIVERSE_DOMAIN);
    let resolved = if configured == TOKEN_URL_TEMPLATE {
        configured.replace("{universe_domain}", universe)
    } else {
        configured.to_owned()
    };
    Url::parse(&resolved).map_err(|source| FederationError::Url {
        field: "token_url",
        source,
    })
}

// google-auth drops an empty form field the same way as an absent one
fn sts_request<R>(
    federation: &AwsFederation<'_, R>,
    subject_token: &SecretString,
    scope: &str,
) -> Result<OutboundRequest, FederationError> {
    let fields = [
        ("grant_type", Some(GRANT_TYPE)),
        ("audience", federation.audience.as_deref()),
        ("scope", Some(scope)),
        ("requested_token_type", Some(REQUESTED_TOKEN_TYPE)),
        ("subject_token", Some(subject_token.expose_secret())),
        (
            "subject_token_type",
            federation.subject_token_type.as_deref(),
        ),
    ];
    let mut form = Serializer::new(String::new());
    for (name, field) in fields {
        if let Some(text) = field.filter(|text| !text.is_empty()) {
            form.append_pair(name, text);
        }
    }
    Ok(OutboundRequest {
        method: Method::POST,
        url: token_endpoint(federation)?,
        query: Vec::new(),
        headers: HeaderMap::new(),
        body: RequestBody::Bytes {
            content_type: HeaderValue::from_static("application/x-www-form-urlencoded"),
            data: form.finish().into_bytes(),
        },
        deadline: REQUEST_DEADLINE,
    })
}

fn impersonation_request(
    scopes: &[String],
    endpoint: &str,
    sts_token: &SecretString,
) -> Result<OutboundRequest, FederationError> {
    let mut authorization = HeaderValue::try_from(format!("Bearer {}", sts_token.expose_secret()))
        .map_err(FederationError::Authorization)?;
    authorization.set_sensitive(true);
    let mut headers = HeaderMap::new();
    headers.insert(AUTHORIZATION, authorization);
    let body = serde_json::to_vec(&ImpersonationBody {
        delegates: None,
        scope: Some(scopes).filter(|names| !names.is_empty()),
        lifetime: IMPERSONATED_LIFETIME,
    })
    .map_err(FederationError::ImpersonationBody)?;
    Ok(OutboundRequest {
        method: Method::POST,
        url: Url::parse(endpoint).map_err(|source| FederationError::Url {
            field: "service_account_impersonation_url",
            source,
        })?,
        query: Vec::new(),
        headers,
        body: RequestBody::Json(body),
        deadline: REQUEST_DEADLINE,
    })
}
