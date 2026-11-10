use serde::Deserialize;
use std::collections::BTreeMap;
#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Input {
    JwtAudienceHint { tokens: Vec<String> },
    WebIdentity(Case),
    SameAccountIrsa(Case),
    CrossAccountIrsa(Case),
}
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct Case {
    pub(super) environment: BTreeMap<String, String>,
    pub(super) token_reference: String,
    pub(super) resolved_token: Option<String>,
    pub(super) role_arn: String,
    pub(super) session_name: String,
    pub(super) irsa_role_arn: String,
    pub(super) token_file: Option<TokenFile>,
    pub(super) external_id: Option<String>,
    pub(super) tags: Vec<ReplyTag>,
    pub(super) max_policy_size: Option<i64>,
    pub(super) reply: Option<WebIdentityReply>,
    pub(super) irsa_reply: Option<WebIdentityReply>,
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum WebIdentityReply {
    Credentials(ReplyCredentials),
    InvalidIdentityToken { message: String },
    MissingPackedPolicySize(ReplyCredentials),
}
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub(crate) struct ReplyCredentials {
    pub(super) access_key_id: String,
    pub(super) secret_access_key: String,
    pub(super) session_token: String,
    pub(super) expiration: String,
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
