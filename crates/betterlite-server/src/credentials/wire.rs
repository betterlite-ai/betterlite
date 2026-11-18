use betterlite_db::identity_rows::Payload;
use serde::Serialize;

#[derive(Debug, Serialize)]
pub(super) struct SuccessResponse {
    pub(super) success: bool,
    pub(super) message: &'static str,
}

#[derive(Debug, Serialize)]
pub(super) struct CredentialView {
    pub(super) credential_name: String,
    pub(super) credential_values: Payload,
    pub(super) credential_info: Payload,
}

#[derive(Debug, Serialize)]
pub(super) struct CredentialListResponse {
    pub(super) success: bool,
    pub(super) credentials: Vec<CredentialView>,
}
