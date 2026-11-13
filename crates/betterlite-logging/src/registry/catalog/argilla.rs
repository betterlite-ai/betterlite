mod adapter;
mod tests;

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rand::SeedableRng as _;
use rand::rngs::{StdRng, SysRng};
use tokio::time::timeout;
use tokio_util::sync::CancellationToken;

use super::{CatalogError, CatalogInputs};
use crate::argilla_logging::logger::SeededRandom;
use crate::argilla_logging::settings::{
    API_KEY_VARIABLE, ArgillaSettings, BASE_URL_VARIABLE, BATCH_SIZE_VARIABLE,
    ConstructorArguments, DATASET_NAME_VARIABLE, SAMPLING_RATE_VARIABLE,
    SYNC_SAMPLING_RATE_VARIABLE, SettingsError,
};
use crate::argilla_logging::transport::PooledTransport;
use crate::batch::logger::{
    BATCH_SIZE_VARIABLE as DEFAULT_BATCH_SIZE_VARIABLE, FLUSH_INTERVAL_VARIABLE,
};
use crate::callback_logger::CallbackLogger;
use adapter::{ArgillaAdapter, Pending};

const CALLBACK_NAME: &str = "argilla";
const ARGILLA_ENVIRONMENT: [&str; 8] = [
    API_KEY_VARIABLE,
    BASE_URL_VARIABLE,
    DATASET_NAME_VARIABLE,
    SAMPLING_RATE_VARIABLE,
    BATCH_SIZE_VARIABLE,
    SYNC_SAMPLING_RATE_VARIABLE,
    DEFAULT_BATCH_SIZE_VARIABLE,
    FLUSH_INTERVAL_VARIABLE,
];

type Production = ArgillaAdapter<PooledTransport, SeededRandom<StdRng>>;

pub fn build(inputs: &CatalogInputs) -> Result<Arc<dyn CallbackLogger>, CatalogError> {
    let transformation = transformation(inputs);
    let settings = ArgillaSettings::from_environment(
        &environment(inputs),
        &ConstructorArguments::default(),
        transformation.as_ref(),
    )
    .map_err(|error| CatalogError::Init {
        message: init_message(error),
    })?;
    let interval = settings.batch.flush_interval;
    let random = StdRng::try_from_rng(&mut SysRng).map_err(|error| CatalogError::Init {
        message: format!("could not seed the Argilla sampler from the OS: {error}"),
    })?;
    let adapter = Arc::new(ArgillaAdapter::new(
        Pending {
            settings,
            transport: PooledTransport {
                client: inputs.http.clone(),
            },
            random: SeededRandom {
                generator: Mutex::new(random),
            },
        },
        inputs.tracker.clone(),
    ));
    spawn_flush_loop(&adapter, interval, inputs.shutdown.clone());
    Ok(adapter)
}

// LiteLLM keeps the map in litellm_settings.argilla_transformation_object; the callback_settings entry carries the same map here
fn transformation(inputs: &CatalogInputs) -> Option<Json> {
    let entry = inputs.callback_settings.get(CALLBACK_NAME)?;
    let mut fields = IndexMap::new();
    for (name, value) in entry.exposed_variables() {
        fields.insert(name, Json::Text(value));
    }
    Some(Json::Object(fields))
}

fn environment(inputs: &CatalogInputs) -> BTreeMap<String, String> {
    ARGILLA_ENVIRONMENT
        .iter()
        .filter_map(|name| {
            inputs
                .env
                .lookup(name)
                .map(|value| ((*name).to_owned(), value))
        })
        .collect()
}

// The catalog logs this text and skips the name, as set_callbacks does for the exception ArgillaLogger.__init__ raises.
fn init_message(error: SettingsError) -> String {
    match error {
        SettingsError::MissingTransformation => {
            "'litellm.argilla_transformation_object' is required, to log your payload to Argilla."
                .to_owned()
        }
        SettingsError::TransformationNotDictionary => {
            "'argilla_transformation_object' must be a dictionary, to log your payload to Argilla."
                .to_owned()
        }
        SettingsError::UnsupportedField { value } => format!(
            "All values in argilla_transformation_object must be a key in SUPPORTED_PAYLOAD_FIELDS, {value} is not a valid key."
        ),
        SettingsError::MissingApiKey => {
            "Invalid Argilla API Key given. _credentials_api_key=None.".to_owned()
        }
        other @ (SettingsError::Batch(_) | SettingsError::BatchSize(_)) => other.to_string(),
    }
}

fn spawn_flush_loop(adapter: &Arc<Production>, interval: Duration, shutdown: CancellationToken) {
    let flush = Arc::clone(adapter);
    let _task = adapter.tracker.spawn(async move {
        run_flush_loop(&flush, interval, shutdown).await;
    });
}

// CustomBatchLogger.periodic_flush started by ArgillaLogger.__init__: the first tick resolves the dataset, then each tick flushes.
async fn run_flush_loop(adapter: &Production, interval: Duration, shutdown: CancellationToken) {
    loop {
        let _woken = timeout(interval, shutdown.cancelled()).await;
        if shutdown.is_cancelled() {
            break;
        }
        let Ok(logger) = adapter.connection.logger().await else {
            tracing::error!("the Argilla logger could not connect; the flush loop stops");
            break;
        };
        let report = logger.flush_queue().await;
        if report.cancelled {
            tracing::warn!(
                batch = report.batch,
                "Argilla flush cancelled; records stay queued"
            );
        }
    }
}
