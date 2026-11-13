use std::sync::Arc;
use std::time::{Duration, SystemTime};

use betterlite_logging::s3_transport::batch_element::S3BatchLoggingElement;
use betterlite_logging::s3_transport::encryption::{
    ServerSideEncryption, resolve_server_side_encryption,
};
use betterlite_logging::s3_transport::option_value::OptionValue;
use betterlite_logging::s3_transport::signing::{CredentialSource, Credentials, StaticCredentials};
use betterlite_logging::s3_transport::transport::{
    S3HttpClient, S3Transport, ThrottleSink, UploadOutcome,
};
use betterlite_logging::s3_transport::url_construction::S3Endpoint;
use betterlite_models::decisions::Json;
use secrecy::SecretString;
use serde_json::json;

use crate::helpers::{Helper, HelperError, HelperResult};

use input::{Input, Operation};
use recording::{FixedClock, Recorded, RecordingSink, ScriptedS3};

mod input;
mod recording;

pub(crate) struct Op;

fn failure(class: &str, message: &str) -> Box<HelperError> {
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: message.to_owned(),
    })
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .start_paused(true)
            .build()
            .map_err(|error| failure("RuntimeError", &error.to_string()))?;
        runtime.block_on(execute(input))
    }
}

async fn execute(input: &Input) -> HelperResult {
    let sink = Arc::new(RecordingSink::default());
    let scripted = Arc::new(ScriptedS3::new(input.responses.clone()));
    let transport = build_transport(input, &scripted, &sink)?;
    let mut outcome = None;
    let mut samples = None;
    let mut object = None;
    match input.operation {
        Operation::Put => {
            let logged = logged_element(input)?;
            outcome = Some(match transport.async_upload_data_to_s3(&logged).await {
                UploadOutcome::Delivered => "delivered",
                UploadOutcome::Retry => "retry",
                UploadOutcome::Dropped => "dropped",
            });
            samples = input.settings.adaptive_concurrency.then(|| sink.samples());
        }
        Operation::Get => {
            let object_key = input
                .object_key
                .as_ref()
                .ok_or_else(|| failure("ValueError", "a get call needs an object_key"))?;
            object = transport.download_object(object_key).await;
        }
    }
    Ok(json!({
        "requests": recorded_requests(input.operation, &scripted),
        "outcome": outcome,
        "samples": samples,
        "object": object,
    }))
}

fn logged_element(input: &Input) -> Result<S3BatchLoggingElement<Json>, Box<HelperError>> {
    let element = input
        .element
        .as_ref()
        .ok_or_else(|| failure("ValueError", "a put call needs an element"))?;
    Ok(S3BatchLoggingElement {
        payload: element.payload.clone().unwrap_or(Json::Null),
        s3_object_key: element.s3_object_key.clone(),
        s3_object_download_filename: element.s3_object_download_filename.clone(),
        body: element.body.clone(),
        content_type: element.content_type,
        retrying_since: None,
    })
}

fn build_transport(
    input: &Input,
    scripted: &Arc<ScriptedS3>,
    sink: &Arc<RecordingSink>,
) -> Result<S3Transport, Box<HelperError>> {
    let client: Arc<dyn S3HttpClient> = Arc::<ScriptedS3>::clone(scripted);
    let throttle = input
        .settings
        .adaptive_concurrency
        .then(|| -> Arc<dyn ThrottleSink> { Arc::<RecordingSink>::clone(sink) });
    let credentials: Arc<dyn CredentialSource> = Arc::new(StaticCredentials {
        credentials: Credentials {
            access_key_id: SecretString::from(input.credentials.access_key_id.clone()),
            secret_access_key: SecretString::from(input.credentials.secret_access_key.clone()),
            session_token: input
                .credentials
                .session_token
                .as_ref()
                .map(|token| SecretString::from(token.clone())),
        },
    });
    let now = SystemTime::UNIX_EPOCH
        .checked_add(Duration::from_secs(input.now))
        .ok_or_else(|| failure("ValueError", "now is too large"))?;
    Ok(S3Transport {
        client,
        endpoint: S3Endpoint {
            bucket: input.settings.bucket_name.clone(),
            region: input.settings.region_name.clone(),
            endpoint_url: input.settings.endpoint_url.clone(),
            virtual_hosted_style: input.settings.use_virtual_hosted_style,
        },
        credentials,
        encryption: encryption(input),
        drop_on_terminal_error: input.settings.drop_on_terminal_error,
        throttle,
        clock: Arc::new(FixedClock(now)),
    })
}

fn recorded_requests(operation: Operation, scripted: &ScriptedS3) -> Vec<Recorded> {
    scripted
        .sent()
        .into_iter()
        .map(|request| Recorded {
            method: request.method.as_str().to_owned(),
            url: request.url,
            body: match operation {
                Operation::Put => Some(String::from_utf8_lossy(&request.body).into_owned()),
                Operation::Get => None,
            },
            headers: request
                .headers
                .into_iter()
                .map(|(name, value)| (name.to_ascii_lowercase(), value))
                .collect(),
        })
        .collect()
}

fn encryption(input: &Input) -> Option<ServerSideEncryption> {
    let algorithm = input
        .settings
        .server_side_encryption
        .as_ref()
        .map(|text| OptionValue::Text(text.clone()));
    let kms_key = input
        .settings
        .sse_kms_key_id
        .as_ref()
        .map(|text| OptionValue::Text(text.clone()));
    resolve_server_side_encryption(algorithm.as_ref(), kms_key.as_ref())
}
