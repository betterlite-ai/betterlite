mod tests;

use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use betterlite_config::sections::settings::JsonValue;
use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use http::Method;
use http::header::{ACCEPT, HeaderMap, HeaderName, HeaderValue};
use url::Url;

use super::failure::{RefreshFailure, RefreshFailureKind};

pub const NON_OBJECT_DETAIL: &str = "the IdP token endpoint returned a non-object response";
pub const UNREACHABLE_DETAIL: &str = "the IdP token endpoint is unreachable";
const JSON_ACCEPT: &str = "application/json";
const FORM_CONTENT_TYPE: &str = "application/x-www-form-urlencoded";

// The token endpoint's answer is a passthrough object: the refresher reads id_token and refresh_token, and the IdP owns every other key.
pub type RefreshResponseBody = BTreeMap<String, JsonValue>;

pub enum GrantAnswer {
    Answered { status: u16, body: Vec<u8> },
    Unreachable,
}

impl fmt::Debug for GrantAnswer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Answered { status, .. } => f
                .debug_struct("Answered")
                .field("status", status)
                .finish_non_exhaustive(),
            Self::Unreachable => f.write_str("Unreachable"),
        }
    }
}

#[async_trait]
pub trait RefreshTransport: Send + Sync {
    async fn post(
        &self,
        url: &str,
        form: &BTreeMap<String, String>,
        headers: &BTreeMap<String, String>,
    ) -> Result<RefreshResponseBody, RefreshFailure>;
}

#[async_trait]
pub trait RefreshGrantPost: Send + Sync {
    async fn post_form(
        &self,
        url: &str,
        form: &BTreeMap<String, String>,
        headers: &BTreeMap<String, String>,
    ) -> GrantAnswer;
}

pub struct GrantPostTransport<P> {
    pub post: P,
}

#[async_trait]
impl<P> RefreshTransport for GrantPostTransport<P>
where
    P: RefreshGrantPost,
{
    async fn post(
        &self,
        url: &str,
        form: &BTreeMap<String, String>,
        headers: &BTreeMap<String, String>,
    ) -> Result<RefreshResponseBody, RefreshFailure> {
        match self.post.post_form(url, form, headers).await {
            GrantAnswer::Unreachable => Err(RefreshFailure {
                kind: RefreshFailureKind::Unavailable,
                detail: UNREACHABLE_DETAIL.to_owned(),
            }),
            GrantAnswer::Answered { status, body } => map_answer(status, &body),
        }
    }
}

// Only a 4xx is settled: a 429 included, because the IdP refusing this grant means the user signs in again, not that betterlite retries.
pub fn map_answer(status: u16, body: &[u8]) -> Result<RefreshResponseBody, RefreshFailure> {
    match status {
        400..=499 => Err(RefreshFailure {
            kind: RefreshFailureKind::Rejected,
            detail: format!("the IdP refused the refresh with status {status}"),
        }),
        200..=299 => {
            serde_json::from_slice::<RefreshResponseBody>(body).map_err(|_not_an_object| {
                RefreshFailure {
                    kind: RefreshFailureKind::Unavailable,
                    detail: NON_OBJECT_DETAIL.to_owned(),
                }
            })
        }
        _ => Err(RefreshFailure {
            kind: RefreshFailureKind::Unavailable,
            detail: format!("the IdP token endpoint answered with status {status}"),
        }),
    }
}

pub struct HttpGrantFormPost {
    pub client: Arc<ProxiedClient>,
    pub deadline: Duration,
}

#[async_trait]
impl RefreshGrantPost for HttpGrantFormPost {
    async fn post_form(
        &self,
        url: &str,
        form: &BTreeMap<String, String>,
        headers: &BTreeMap<String, String>,
    ) -> GrantAnswer {
        let Ok(endpoint) = Url::parse(url) else {
            tracing::warn!("ID-JAG refresh POST failed: the token endpoint is not a URL");
            return GrantAnswer::Unreachable;
        };
        let Some(request) = grant_form_request(&endpoint, form, headers, self.deadline) else {
            tracing::warn!("ID-JAG refresh POST could not be built from the client auth");
            return GrantAnswer::Unreachable;
        };
        let Ok(sent) = self.client.send_following(request).await else {
            tracing::warn!("ID-JAG refresh POST to the IdP token endpoint failed");
            return GrantAnswer::Unreachable;
        };
        let status = sent.response.status().as_u16();
        if !(200..=299).contains(&status) {
            return GrantAnswer::Answered {
                status,
                body: Vec::new(),
            };
        }
        match sent.buffered(self.client.limits.body_bytes).await {
            Ok(response) => GrantAnswer::Answered {
                status,
                body: response.body,
            },
            Err(_oversized_or_broken) => {
                tracing::warn!("ID-JAG refresh POST failed while reading the IdP's answer");
                GrantAnswer::Unreachable
            }
        }
    }
}

fn grant_form_request(
    endpoint: &Url,
    form: &BTreeMap<String, String>,
    headers: &BTreeMap<String, String>,
    deadline: Duration,
) -> Option<OutboundRequest> {
    let mut encoded = form_urlencoded::Serializer::new(String::new());
    for (name, value) in form {
        encoded.append_pair(name, value);
    }
    let mut map = HeaderMap::with_capacity(headers.len().saturating_add(1));
    map.insert(ACCEPT, HeaderValue::from_static(JSON_ACCEPT));
    for (name, value) in headers {
        let header_name = HeaderName::try_from(name.as_str()).ok()?;
        let mut header_value = HeaderValue::try_from(value.as_str()).ok()?;
        header_value.set_sensitive(true);
        map.insert(header_name, header_value);
    }
    Some(OutboundRequest {
        method: Method::POST,
        url: endpoint.clone(),
        query: Vec::new(),
        headers: map,
        body: RequestBody::Bytes {
            content_type: HeaderValue::from_static(FORM_CONTENT_TYPE),
            data: encoded.finish().into_bytes(),
        },
        deadline,
    })
}
