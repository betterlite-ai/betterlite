use std::collections::BTreeMap;
use std::fmt;

use serde::Serialize;

use crate::gateway_authorize::no_cache_headers;
use crate::gateway_dcr::oauth_error::OauthError;
use crate::types::{SessionAudience, SessionTokenKind};

#[derive(Debug)]
pub enum TokenReply {
    // the body is boxed so the success and error variants stay within variant_size_differences
    Json {
        status: u16,
        body: Box<TokenBody>,
        headers: Vec<(String, String)>,
    },
    Error(OauthError),
}

#[derive(Debug)]
pub enum TokenBody {
    SessionPair(SessionPairBody),
    ProxyCredential(ProxyCredentialBody),
    Revoked(RevokedBody),
    Introspection(IntrospectionBody),
}

// the token fields are plain strings for the wire, so Debug is handwritten and redacts them
#[derive(Serialize)]
pub struct SessionPairBody {
    pub access_token: String,
    pub token_type: &'static str,
    pub expires_in: i64,
    pub refresh_token: String,
}

const REDACTED: &str = "<redacted>";

impl fmt::Debug for SessionPairBody {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut fields = f.debug_struct("SessionPairBody");
        fields.field("access_token", &REDACTED);
        fields.field("token_type", &self.token_type);
        fields.field("expires_in", &self.expires_in);
        fields.field("refresh_token", &REDACTED);
        fields.finish()
    }
}

fn redacted_debug(
    formatter: &mut fmt::Formatter<'_>,
    name: &str,
    extra: &[(&str, &dyn fmt::Debug)],
) -> fmt::Result {
    let mut fields = formatter.debug_struct(name);
    fields.field("access_token", &REDACTED);
    fields.field("refresh_token", &REDACTED);
    for (field, value) in extra {
        fields.field(field, value);
    }
    fields.finish()
}

#[derive(Serialize)]
pub struct ProxyCredentialBody {
    pub access_token: String,
    pub token_type: &'static str,
    pub expires_in: u64,
    pub refresh_token: String,
    pub user_id: String,
    pub team_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub issued_token_type: Option<&'static str>,
}

impl fmt::Debug for ProxyCredentialBody {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        redacted_debug(
            f,
            "ProxyCredentialBody",
            &[
                ("token_type", &self.token_type),
                ("expires_in", &self.expires_in),
                ("user_id", &self.user_id),
                ("team_id", &self.team_id),
                ("issued_token_type", &self.issued_token_type),
            ],
        )
    }
}

// the transparent empty map is how the pinned body's "{}" survives serde's derives
#[derive(Debug, Default, Serialize)]
#[serde(transparent)]
pub struct RevokedBody(BTreeMap<String, String>);

#[derive(Debug, Serialize)]
pub struct IntrospectionBody {
    pub active: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub iss: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sub: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jti: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub iat: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exp: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<SessionTokenKind>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_type: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub team_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource_server_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub audience: Option<SessionAudience>,
}

#[must_use]
pub fn json_reply(status: u16, body: TokenBody) -> TokenReply {
    TokenReply::Json {
        status,
        body: Box::new(body),
        headers: no_cache_headers(),
    }
}
