use betterlite_db::identity_rows::Payload;
use serde::Serialize;

#[derive(Debug, Serialize)]
pub(super) struct SuccessResponse {
    pub(super) success: bool,
    pub(super) message: &'static str,
}

#[derive(Debug, Serialize)]
pub(super) struct CredentialView {
    #[serde(rename = "credential_name")]
    pub(super) name: String,
    #[serde(rename = "credential_values")]
    pub(super) values: Payload,
    #[serde(rename = "credential_info")]
    pub(super) info: Payload,
}

#[derive(Debug, Serialize)]
pub(super) struct CredentialListResponse {
    pub(super) success: bool,
    pub(super) credentials: Vec<CredentialView>,
}
