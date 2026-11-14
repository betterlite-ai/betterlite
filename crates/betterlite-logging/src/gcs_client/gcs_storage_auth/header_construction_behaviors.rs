use secrecy::SecretString;

use crate::gcs_mock::auth::{EnsureAccessToken, VertexAuth, VertexAuthProvider};
use crate::gcs_mock::client::GcsHttpClient;

use super::error::GcsClientError;
use super::token_request_behaviors::{header_error, token_headers};
use super::{GcsBucketBase, RequestHeaders, VertexInstance};

pub async fn construct_request_headers<Auth, Http>(
    base: &GcsBucketBase<Auth, Http>,
    bucket_name: &str,
    service_account_json: Option<&SecretString>,
    vertex_instance: Option<&VertexInstance<Auth>>,
) -> Result<RequestHeaders, GcsClientError<Auth, Http>>
where
    Auth: VertexAuth + Clone,
    Http: GcsHttpClient,
{
    let instance = vertex_instance.unwrap_or(&base.auth);
    let access = instance
        .ensure_access_token_async(EnsureAccessToken {
            credentials: service_account_json.cloned(),
            project_id: None,
            custom_llm_provider: VertexAuthProvider::VertexAi,
        })
        .await
        .map_err(|source| header_error(bucket_name, source))?;
    token_headers(instance, access, bucket_name, service_account_json)
}

mod tests;
