use std::collections::BTreeMap;

use betterlite_logging::gcs_client::gcs_storage_auth::SERVICE_ACCOUNT_ENV;
use betterlite_vertex::vertex_file_content::settings::{
    ReadDeployment, ReadEnvironment, vertex_file_read_settings,
};
use betterlite_vertex::vertex_gcs_access::bucket::{GCS_BATCH_BUCKET_ENV, GCS_BUCKET_ENV};
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::{Value, json};

use super::input::Input;

pub(super) fn run(input: &Input) -> Value {
    let deployment = ReadDeployment {
        gcs_bucket_name: input.params.get("gcs_bucket_name").cloned(),
        bucket_name: input.params.get("bucket_name").cloned(),
        vertex_credentials: input.params.get("vertex_credentials").cloned(),
    };
    let environment = read_environment(&input.environment);
    let settings = vertex_file_read_settings(&deployment, input.argument.as_ref(), &environment);
    json!({
        "bucket_name": settings.bucket_name,
        "path_service_account": settings
            .service_account
            .map(|secret| secret.expose_secret().to_owned()),
    })
}

fn read_environment(values: &BTreeMap<String, String>) -> ReadEnvironment {
    ReadEnvironment {
        batch_bucket_name: values.get(GCS_BATCH_BUCKET_ENV).cloned(),
        bucket_name: values.get(GCS_BUCKET_ENV).cloned(),
        path_service_account: values
            .get(SERVICE_ACCOUNT_ENV)
            .cloned()
            .map(SecretString::from),
    }
}
