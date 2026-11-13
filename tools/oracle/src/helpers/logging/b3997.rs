use std::fmt::Display;
use std::sync::{Arc, Mutex};

use betterlite_logging::gcs_client::gcs_storage_auth::GcsBucketBase;
use betterlite_logging::gcs_client::gcs_storage_auth::environment_source::EnvironmentSource;
use betterlite_logging::gcs_client::gcs_storage_auth::initialization_behaviors::initialize;
use betterlite_logging::gcs_mock::request::GcsRequest;

use crate::helpers::{Helper, HelperError, HelperResult};

mod cases;
mod input;
mod output;
mod recording;
mod tests;

use self::cases::{
    async_headers_case, initialize_case, logging_config_case, sync_headers_case, upload_case,
    vertex_instance_case,
};
use self::input::{Action, Input};
use self::recording::{FakeAuth, RecordingClient};

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .map_err(|error| failure("RuntimeError", error))?;
        runtime.block_on(execute(input))
    }
}

async fn execute(input: &Input) -> HelperResult {
    match input.action {
        Action::Initialize => initialize_case(input).await,
        Action::AsyncHeaders => async_headers_case(input).await,
        Action::SyncHeaders => sync_headers_case(input),
        Action::Upload => upload_case(input).await,
        Action::LoggingConfig => logging_config_case(input).await,
        Action::VertexInstance => vertex_instance_case(input).await,
    }
}

pub(super) struct Session {
    pub(super) base: GcsBucketBase<FakeAuth, RecordingClient>,
    pub(super) auth: FakeAuth,
    pub(super) sent: Arc<Mutex<Vec<GcsRequest>>>,
}

pub(super) fn open(input: &Input) -> Result<Session, Box<HelperError>> {
    let auth = FakeAuth::default();
    let sent = Arc::new(Mutex::new(Vec::new()));
    let body = input.body.clone().unwrap_or_default();
    let client = RecordingClient::new(Arc::clone(&sent), input.status.unwrap_or(200), body);
    let environment: Arc<dyn EnvironmentSource> = Arc::new(input.environment.clone());
    let base = initialize(environment, input.bucket.clone(), auth.clone(), client)
        .map_err(|error| failure("ValueError", error))?;
    Ok(Session { base, auth, sent })
}

pub(super) fn bucket_name(input: &Input) -> &str {
    input.bucket.as_deref().unwrap_or("bucket")
}

pub(super) fn required_text<'text>(
    value: Option<&'text str>,
    name: &str,
) -> Result<&'text str, Box<HelperError>> {
    value.ok_or_else(|| failure("ValueError", name))
}

pub(super) fn failure<Message>(class: &str, message: Message) -> Box<HelperError>
where
    Message: Display,
{
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: message.to_string(),
    })
}
