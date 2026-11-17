use async_trait::async_trait;
use betterlite_prompts::gitlab_client::payload::decode_text;
use http::header::{AUTHORIZATION, CONTENT_TYPE};
use http::{HeaderMap, HeaderValue, Method};
use secrecy::SecretString;

use crate::databricks_auth::oauth::{OauthReply, OauthTokenRequest, OauthTokenSource};

use super::transport::{PostTransport, WireRequest};

// The token fetch LiteLLM's DatabricksBase makes with requests.post, over the dispatch's transport
pub struct OauthM2m<'transport> {
    pub transport: &'transport dyn PostTransport,
}

#[async_trait]
impl OauthTokenSource for OauthM2m<'_> {
    async fn send(&self, request: &OauthTokenRequest) -> OauthReply {
        let mut headers = HeaderMap::new();
        let Ok(content_type) = HeaderValue::from_str(&request.content_type) else {
            return OauthReply::Transport(
                "the content type is not a valid header value".to_owned(),
            );
        };
        let Ok(authorization) = HeaderValue::from_str(&request.authorization) else {
            return OauthReply::Transport(
                "the OAuth authorization is not a valid header value".to_owned(),
            );
        };
        headers.insert(CONTENT_TYPE, content_type);
        headers.insert(AUTHORIZATION, authorization);
        let wire = WireRequest {
            method: Method::POST,
            url: &request.url,
            headers: &headers,
            body: request.body.as_bytes(),
            deadline: request.timeout,
        };
        match self.transport.post(&wire).await {
            Ok(response) => OauthReply::Reply {
                status: response.status,
                body: SecretString::from(decode_text(&response.body)),
            },
            Err(error) => OauthReply::Transport(error.to_string()),
        }
    }
}
