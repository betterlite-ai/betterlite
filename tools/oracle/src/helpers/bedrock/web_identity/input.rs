use std::collections::BTreeMap;

use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Input {
    JwtAudienceHint { tokens: Vec<String> },
    MalformedToken { tokens: Vec<String> },
    WebIdentityAssumeRole(Case),
    SecureTransportSessionPolicy(Case),
    InvalidIdentityTokenError(Case),
    StsCredentialsMissing(Case),
    PackedPolicyWarningAndTtl(Case),
    SameAccountIrsa(Case),
    CrossAccountIrsa(Case),
}

#[derive(Debug, Deserialize)]
pub(crate) struct Case {
    #[serde(default)]
    pub(super) environment: BTreeMap<String, String>,
    #[serde(default)]
    pub(super) token_reference: String,
    #[serde(default)]
    pub(super) resolved_token: Option<String>,
    #[serde(default)]
    pub(super) role_arn: String,
    #[serde(default)]
    pub(super) session_name: String,
    #[serde(default)]
    pub(super) irsa_role_arn: String,
    #[serde(default)]
    pub(super) token_file: Option<TokenFile>,
    #[serde(default)]
    pub(super) external_id: Option<String>,
    #[serde(default)]
    pub(super) tags: Vec<ReplyTag>,
    #[serde(default)]
    pub(super) region_name: Option<String>,
    #[serde(default)]
    pub(super) sts_endpoint: Option<String>,
    #[serde(default)]
    pub(super) max_policy_size: Option<i64>,
    #[serde(default)]
    pub(super) verify: Option<bool>,
    #[serde(default)]
    pub(super) reply: Option<Reply>,
    #[serde(default)]
    pub(super) irsa_reply: Option<ReplyCredentials>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub(crate) enum Reply {
    Web(WebIdentityReply),
    Credentials(ReplyCredentials),
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum WebIdentityReply {
    Credentials(ReplyCredentials),
    InvalidIdentityToken { message: String },
    MissingPackedPolicySize(ReplyCredentials),
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct ReplyCredentials {
    pub(super) access_key_id: String,
    pub(super) secret_access_key: String,
    pub(super) session_token: String,
    pub(super) expiration: String,
    #[serde(default)]
    pub(super) packed_policy_size: Option<i32>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct ReplyTag {
    pub(super) key: String,
    pub(super) value: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct TokenFile {
    pub(super) path: String,
    pub(super) content: String,
}
