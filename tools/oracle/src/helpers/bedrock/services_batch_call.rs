use std::collections::BTreeMap;
use std::time::{Duration, SystemTime};

use betterlite_bedrock::batch_call::async_status::{AsyncStatusCall, retrieve_async_status};
use betterlite_bedrock::batch_call::cancel::{CancelBatchCall, cancel_batch};
use betterlite_bedrock::batch_call::region::DEFAULT_REGION;
use betterlite_bedrock::batch_call::status::{JobStatusCall, retrieve_model_job_status};
use betterlite_bedrock::bedrock_embedding_io::credentials::ManagedRegionWorker;
use betterlite_bedrock::embedding_jobs::status::StatusServices;
use betterlite_bedrock::signing::executor::AwsSigningExecutor;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_secret_managers::secret_lookup::SecretLookup;
use serde::Deserialize;
use serde_json::Value;
use serde_json::json;
use serde_json::value::RawValue;
use tokio::runtime::{Builder, Runtime};

mod fixtures;
mod secrets;
mod tests;
mod view;

use fixtures::{AsyncFixture, PlaneFactory, Source};
use secrets::{async_parameters, credentials_of, fixture_secrets};

const CLOCK_SECONDS: u64 = 1_700_000_000;

#[derive(Debug, Deserialize)]
pub(crate) struct Input {
    operation: Operation,
    batch_id: String,
    #[serde(default)]
    region: Option<String>,
    credentials: CredentialInput,
    #[serde(default)]
    replies: Vec<ReplyInput>,
    #[serde(default)]
    reply: Option<ReplyInput>,
    #[serde(default)]
    logging: bool,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Operation {
    Cancel,
    ModelJobStatus,
    AsyncStatus,
}

#[derive(Clone, Debug, Deserialize)]
pub(crate) struct CredentialInput {
    access_key_id: String,
    secret_access_key: String,
    #[serde(default)]
    session_token: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub(crate) struct ReplyInput {
    status: u16,
    #[serde(default)]
    headers: BTreeMap<String, String>,
    #[serde(default)]
    body: String,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input.operation {
            Operation::Cancel => control_plane(input, true),
            Operation::ModelJobStatus => control_plane(input, false),
            Operation::AsyncStatus => async_status(input),
        }
    }
}

struct Fixed {
    runtime: Runtime,
    kwargs: BTreeMap<String, Box<RawValue>>,
    now: SystemTime,
}

impl Fixed {
    fn new() -> Result<Self, Box<HelperError>> {
        let mut builder = Builder::new_current_thread();
        builder.enable_all();
        Ok(Self {
            runtime: builder.build().or_value_error()?,
            kwargs: BTreeMap::new(),
            now: epoch()?,
        })
    }
}

fn control_plane(input: &Input, cancel: bool) -> HelperResult {
    let fixed = Fixed::new()?;
    let source = Source::new(credentials_of(&input.credentials));
    let factory = PlaneFactory::new(input.replies.clone());
    let mut events = Vec::new();
    let result = view::with_events(input.logging, &mut events, |logger| {
        if cancel {
            fixed.runtime.block_on(cancel_batch(
                CancelBatchCall {
                    batch_id: &input.batch_id,
                    aws_region_name: input.region.as_deref(),
                    kwargs: &fixed.kwargs,
                    now: fixed.now,
                },
                &source,
                &factory,
                logger,
            ))
        } else {
            fixed.runtime.block_on(retrieve_model_job_status(
                JobStatusCall {
                    batch_id: &input.batch_id,
                    aws_region_name: input.region.as_deref(),
                    kwargs: &fixed.kwargs,
                    now: fixed.now,
                },
                &source,
                &factory,
                logger,
            ))
        }
    });
    let (batch, error) = view::outcome(result);
    Ok(json!({
        "batch": batch,
        "error": error,
        "calls": factory.calls(&input.batch_id),
        "resolved": source.regions(),
        "requests": [],
        "log": events,
    }))
}

fn async_status(input: &Input) -> HelperResult {
    let reply = input
        .reply
        .as_ref()
        .ok_or_else(|| "the async scenario needs one reply".to_owned())
        .or_value_error()?;
    let fixed = Fixed::new()?;
    let source = Source::new(credentials_of(&input.credentials));
    let secrets = fixture_secrets();
    let factory = AsyncFixture::new(reply);
    let (batch, error) = async_outcome(&fixed, input, &source, &secrets, &factory)?;
    Ok(json!({
        "batch": batch,
        "error": error,
        "calls": [],
        "resolved": source.regions(),
        "requests": factory.urls(),
        "log": [],
    }))
}

fn async_outcome(
    fixed: &Fixed,
    input: &Input,
    source: &Source,
    secrets: &SecretLookup,
    factory: &AsyncFixture,
) -> Result<(Value, Value), Box<HelperError>> {
    let mut parameters = async_parameters(&input.credentials)?;
    let region_worker = ManagedRegionWorker::new().or_value_error()?;
    let signing = AwsSigningExecutor::default();
    let services = StatusServices {
        source,
        secrets,
        region_worker: &region_worker,
        factory,
        signing: &signing,
    };
    let environment = BTreeMap::new();
    let region = input.region.as_deref().unwrap_or(DEFAULT_REGION);
    let result = fixed.runtime.block_on(retrieve_async_status(
        AsyncStatusCall {
            invocation_arn: &input.batch_id,
            region,
            optional_params: &mut parameters,
            environment: &environment,
            now: fixed.now,
        },
        &services,
        None,
    ));
    Ok(view::outcome(result))
}

fn epoch() -> Result<SystemTime, Box<HelperError>> {
    let duration = Duration::from_secs(CLOCK_SECONDS);
    SystemTime::UNIX_EPOCH
        .checked_add(duration)
        .ok_or_else(|| "the fixture clock is out of range".to_owned())
        .or_value_error()
}
