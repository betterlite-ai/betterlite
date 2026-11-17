use betterlite_bedrock::s3_access::TrustedModelCredentials;
use betterlite_logging::gcs_client::gcs_storage_auth::GcsBucketBase;
use betterlite_logging::gcs_client::gcs_storage_auth::SERVICE_ACCOUNT_ENV;
use betterlite_logging::gcs_client::gcs_storage_auth::config_resolution_behaviors::logging_config;
use betterlite_logging::gcs_client::gcs_storage_auth::download_behaviors::download_gcs_object;
use betterlite_logging::gcs_client::gcs_storage_auth::environment_source::EnvironmentSource;
use betterlite_logging::gcs_mock::auth::VertexAuth;
use betterlite_logging::gcs_mock::client::GcsHttpClient;
use betterlite_logging::types::gcs_storage_auth::type_definitions::GcsLoggingKwargs;
use betterlite_models::decisions::Json;
use betterlite_openai::files::records::{BinaryContent, FileContentRequest};
use betterlite_openai::files::stream::flatten_headers;
use http::header::{CONTENT_LENGTH, CONTENT_TYPE, HeaderMap, HeaderValue};
use indexmap::IndexMap;
use percent_encoding::percent_decode_str;
use secrecy::{ExposeSecret as _, SecretString};

use crate::vertex_batch_download::records::ConversionContext;
// ast-grep-ignore: no-long-names
use crate::vertex_batch_download::records::transform_file_content_response;
use crate::vertex_file_content::settings::{
    ReadDeployment, ReadEnvironment, vertex_file_read_settings,
};
use crate::vertex_gcs_access::bucket::{GCS_BATCH_BUCKET_ENV, GCS_BUCKET_ENV};
use crate::vertex_gcs_access::decode::decode_vertex_gcs_id;

pub mod error;
mod tests;

use self::error::FileContentError;

const SUCCESS_STATUS: u16 = 200;
const OCTET_STREAM: &str = "application/octet-stream";
const BUCKET_FIELD: &str = "gcs_bucket_name";
const SERVICE_ACCOUNT_FIELD: &str = "gcs_path_service_account";

// one file content read: the request, the deployment values, the caller's credentials, the trusted record and the conversion switch
pub struct ContentRead<'read> {
    pub request: &'read FileContentRequest,
    pub deployment: &'read ReadDeployment,
    pub credentials: Option<&'read Json>,
    pub trusted: &'read TrustedModelCredentials,
    pub disabled: bool,
}

fn read_environment(environment: &dyn EnvironmentSource) -> ReadEnvironment {
    ReadEnvironment {
        batch_bucket_name: environment.get(GCS_BATCH_BUCKET_ENV),
        bucket_name: environment.get(GCS_BUCKET_ENV),
        path_service_account: environment.get(SERVICE_ACCOUNT_ENV).map(SecretString::from),
    }
}

fn logging_kwargs(
    bucket_name: Option<&str>,
    service_account: Option<&SecretString>,
) -> GcsLoggingKwargs {
    let mut fields = IndexMap::new();
    fields.insert(
        BUCKET_FIELD.to_owned(),
        bucket_name.map_or(Json::Null, |name| Json::Text(name.to_owned())),
    );
    fields.insert(
        SERVICE_ACCOUNT_FIELD.to_owned(),
        service_account.map_or(Json::Null, |secret| {
            Json::Text(secret.expose_secret().to_owned())
        }),
    );
    GcsLoggingKwargs {
        dynamic_params: Some(Json::Object(fields)),
    }
}

fn object_headers(length: usize) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static(OCTET_STREAM));
    headers.insert(CONTENT_LENGTH, HeaderValue::from(length));
    headers
}

// the sync entry point awaits this one operation on its own runtime, so the read stays a plain future that never blocks
pub async fn file_content<Auth, Http>(
    base: &GcsBucketBase<Auth, Http>,
    read: &ContentRead<'_>,
    state: &mut ConversionContext,
) -> Result<BinaryContent, FileContentError<Auth, Http>>
where
    Auth: VertexAuth + Clone,
    Http: GcsHttpClient,
{
    let file_id = read.request.file_id.as_str();
    if file_id.is_empty() {
        return Err(FileContentError::MissingFileId);
    }
    let decoded_file_id = percent_decode_str(file_id).decode_utf8_lossy().into_owned();
    let settings = vertex_file_read_settings(
        read.deployment,
        read.credentials,
        &read_environment(base.environment.as_ref()),
    );
    let kwargs = logging_kwargs(
        settings.bucket_name.as_deref(),
        settings.service_account.as_ref(),
    );
    let config = logging_config(base, &kwargs).await?;
    let object = decode_vertex_gcs_id(file_id.as_bytes(), &config.bucket_name, read.trusted)?;
    let download_kwargs = logging_kwargs(
        Some(object.bucket.as_str()),
        config.path_service_account.as_ref(),
    );
    let Some(content) = download_gcs_object(base, &object.encoded_object, &download_kwargs).await
    else {
        return Err(FileContentError::Download {
            file_id: decoded_file_id,
        });
    };
    let headers = object_headers(content.len());
    // the transform returns the stored row untouched when LiteLLM's disable_vertex_batch_output_transformation is on
    let converted =
        // ast-grep-ignore: no-long-names
        transform_file_content_response(&content, &decoded_file_id, &headers, read.disabled, state);
    Ok(BinaryContent {
        status: SUCCESS_STATUS,
        headers: flatten_headers(&converted.headers),
        body: converted.content,
    })
}
