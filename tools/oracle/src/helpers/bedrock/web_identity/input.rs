use std::collections::BTreeMap;

use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Input {
    JwtAudienceHint { tokens: Vec<String> },
    MalformedToken { tokens: Vec<String> },
    WebIdentityAssumeRole(WebIdentityCase),
    SecureTransportSessionPolicy(WebIdentityCase),
    InvalidIdentityTokenError(WebIdentityCase),
    StsCredentialsMissing(WebIdentityCase),
    PackedPolicyWarningAndTtl(WebIdentityCase),
    SameAccountIrsa(SameAccountCase),
    CrossAccountIrsa(CrossAccountCase),
}

#[derive(Debug, Deserialize)]
pub(crate) struct WebIdentityCase {
    #[serde(default)]
    pub(super) environment: BTreeMap<String, String>,
    pub(super) token_reference: String,
    #[serde(default)]
    pub(super) resolved_token: Option<String>,
    pub(super) role_arn: String,
    pub(super) session_name: String,
    #[serde(default)]
    pub(super) region_name: Option<String>,
    #[serde(default)]
    pub(super) sts_endpoint: Option<String>,
    #[serde(default)]
    pub(super) external_id: Option<String>,
    #[serde(default)]
    pub(super) max_policy_size: Option<i64>,
    #[serde(default)]
    pub(super) verify: Option<bool>,
    pub(super) reply: WebIdentityReply,
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
pub(crate) struct SameAccountCase {
    #[serde(default)]
    pub(super) environment: BTreeMap<String, String>,
    pub(super) role_arn: String,
    pub(super) session_name: String,
    #[serde(default)]
    pub(super) external_id: Option<String>,
    #[serde(default)]
    pub(super) tags: Vec<ReplyTag>,
    #[serde(default)]
    pub(super) region_name: Option<String>,
    #[serde(default)]
    pub(super) sts_endpoint: Option<String>,
    #[serde(default)]
    pub(super) verify: Option<bool>,
    pub(super) reply: ReplyCredentials,
}

#[derive(Debug, Deserialize)]
pub(crate) struct CrossAccountCase {
    #[serde(default)]
    pub(super) environment: BTreeMap<String, String>,
    pub(super) irsa_role_arn: String,
    pub(super) role_arn: String,
    pub(super) session_name: String,
    pub(super) token_file: TokenFile,
    #[serde(default)]
    pub(super) external_id: Option<String>,
    #[serde(default)]
    pub(super) tags: Vec<ReplyTag>,
    #[serde(default)]
    pub(super) region_name: Option<String>,
    #[serde(default)]
    pub(super) sts_endpoint: Option<String>,
    #[serde(default)]
    pub(super) verify: Option<bool>,
    pub(super) irsa_reply: ReplyCredentials,
    pub(super) reply: ReplyCredentials,
}

#[derive(Debug, Deserialize)]
pub(crate) struct TokenFile {
    pub(super) path: String,
    pub(super) content: String,
}
