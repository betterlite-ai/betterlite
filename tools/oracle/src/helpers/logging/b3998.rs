use std::sync::Arc;

use betterlite_logging::gcs_client::gcs_storage_auth::GcsBucketBase;
use betterlite_logging::gcs_client::gcs_storage_auth::environment_source::EnvironmentSource;
use betterlite_logging::gcs_client::gcs_storage_auth::initialization_behaviors::initialize;
use betterlite_logging::gcs_logger::gcs_bucket_logging::initialization_behaviors::start;
use betterlite_logging::gcs_logger::gcs_bucket_logging::{GcsBucketLogger, GcsDeps};
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

mod input;
mod recording;
mod steps;
mod tests;

use self::input::Input;
use self::recording::{FixedAuth, FixedClock, FixedIds, RecordingClient, requests_json};
use self::steps::run_step;
use std::fmt::Display;
use tokio::runtime::Builder;

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = Builder::new_current_thread()
            .enable_time()
            .build()
            .map_err(|error| failure("RuntimeError", error))?;
        runtime.block_on(drive(input))
    }
}

struct Session {
    logger: Arc<GcsBucketLogger<FixedAuth, RecordingClient>>,
    client: RecordingClient,
    shutdown: CancellationToken,
    tracker: TaskTracker,
}

impl Session {
    // the periodic flush holds the tracker, so it has to end before the requests are read
    async fn finish(&self) {
        self.shutdown.cancel();
        self.tracker.close();
        self.tracker.wait().await;
    }
}

async fn drive(input: &Input) -> HelperResult {
    let client = RecordingClient::new(&input.replies);
    let base = open(input, client.clone())?;
    let tracker = TaskTracker::new();
    let shutdown = CancellationToken::new();
    let logger = start(
        base,
        GcsDeps {
            clock: Arc::new(FixedClock),
            ids: Arc::new(FixedIds),
            tasks: Some(tracker.clone()),
            shutdown: shutdown.clone(),
        },
    )
    .map_err(|error| failure("ValueError", error))?;
    let session = Session {
        logger,
        client,
        shutdown,
        tracker,
    };
    let results = run_steps(input, &session).await;
    session.finish().await;
    let steps = results?;
    let requests = requests_json(&session.client).or_value_error()?;
    Ok(json!({"steps": steps, "requests": requests}))
}

async fn run_steps(input: &Input, session: &Session) -> Result<Vec<Value>, Box<HelperError>> {
    let mut results = Vec::with_capacity(input.steps.len());
    for step in &input.steps {
        results.push(run_step(step, input, session).await?);
    }
    Ok(results)
}

fn open(
    input: &Input,
    client: RecordingClient,
) -> Result<GcsBucketBase<FixedAuth, RecordingClient>, Box<HelperError>> {
    let environment: Arc<dyn EnvironmentSource> = Arc::new(input.environment.clone());
    initialize(environment, input.bucket.clone(), FixedAuth, client)
        .map_err(|error| failure("ValueError", error))
}

fn failure<Message>(class: &str, message: Message) -> Box<HelperError>
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
