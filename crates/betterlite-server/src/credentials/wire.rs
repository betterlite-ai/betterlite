use betterlite_db::resource_rows::credential::CredentialItem;
use serde::Serialize;

#[derive(Debug, Serialize)]
pub(super) struct SuccessResponse {
    pub(super) success: bool,
    pub(super) message: &'static str,
}

#[derive(Debug, Serialize)]
pub(super) struct CredentialListResponse {
    pub(super) success: bool,
    pub(super) credentials: Vec<CredentialItem>,
}
