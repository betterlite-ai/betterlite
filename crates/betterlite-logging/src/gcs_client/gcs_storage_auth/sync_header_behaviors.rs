use secrecy::SecretString;

use crate::gcs_mock::auth::{EnsureAccessToken, VertexAuth, VertexAuthProvider};
use crate::gcs_mock::client::GcsHttpClient;

use super::error::GcsClientError;
use super::token_request_behaviors::{header_error, token_headers};
use super::{GcsBucketBase, PROJECT_ID_ENV, RequestHeaders};

pub fn sync_construct_request_headers<Auth, Http>(
    base: &GcsBucketBase<Auth, Http>,
    bucket_name: &str,
    service_account_path: Option<&SecretString>,
) -> Result<RequestHeaders, GcsClientError<Auth, Http>>
where
    Auth: VertexAuth + Clone,
    Http: GcsHttpClient,
{
    let project_id = base.environment.get(PROJECT_ID_ENV);
    let request = EnsureAccessToken {
        credentials: service_account_path.cloned(),
        project_id,
        custom_llm_provider: VertexAuthProvider::VertexAi,
    };
    let access = base
        .auth
        .ensure_access_token(request)
        .map_err(|source| header_error(bucket_name, source))?;
    token_headers(&base.auth, access, bucket_name, service_account_path)
}
