mod store;

use std::future::Future;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_server::mcp_rotation::rotation::Rotation;
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::{Value, json};
use tracing::subscriber::set_default;
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};
use tracing_subscriber::registry;

use self::store::{cipher, prepare_store};
use crate::helpers::{Helper, HelperError, HelperResult};

pub(crate) struct Op;

fn helper_error(class: &str, message: &str) -> Box<HelperError> {
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: message.to_owned(),
    })
}

#[derive(Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Function {
    RotateServers,
    RotateUserCredentials,
    RotateUserEnvVars,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub(crate) function: Function,
    pub(crate) salt_key: String,
    pub(crate) new_master_key: String,
    pub(crate) nonce_hex: String,
    #[serde(default)]
    pub(crate) touched_by: String,
    #[serde(default)]
    pub(crate) servers: Vec<Value>,
    #[serde(default)]
    pub(crate) oauth_clients: Vec<Value>,
    #[serde(default)]
    pub(crate) rows: Vec<Value>,
    #[serde(default)]
    pub(crate) fail_at_write: Option<usize>,
}

type Levels = Arc<Mutex<Vec<Value>>>;

struct LevelCapture {
    levels: Levels,
}

impl<S: Subscriber> Layer<S> for LevelCapture {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, S>) {
        let level = *event.metadata().level();
        if level > Level::INFO {
            return;
        }
        let name = if level == Level::ERROR {
            "error"
        } else if level == Level::WARN {
            "warn"
        } else {
            "info"
        };
        self.levels
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Value::String(name.to_owned()));
    }
}

fn block_on<F: Future>(future: F) -> Result<F::Output, Box<HelperError>> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .map_err(|source| helper_error("RuntimeError", &source.to_string()))?;
    Ok(runtime.block_on(future))
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let cipher = cipher(&input.salt_key, &input.nonce_hex)?;
        let key = SecretString::from(input.new_master_key.clone());
        let recorder = Arc::new(Mutex::new(Vec::new()));
        let store = prepare_store(input, &recorder)?;
        let levels: Levels = Arc::new(Mutex::new(Vec::new()));
        let layer = LevelCapture {
            levels: Arc::clone(&levels),
        };
        let guard = set_default(registry().with(layer));
        let rotation = Rotation {
            store: &store,
            cipher: &cipher,
            new_key: &key,
        };
        let result = match input.function {
            Function::RotateServers => block_on(rotation.servers(&input.touched_by))?,
            Function::RotateUserCredentials => block_on(rotation.user_credentials())?,
            Function::RotateUserEnvVars => block_on(rotation.user_env_vars())?,
        };
        drop(guard);
        let writes = recorder
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        let logs = levels
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        Ok(json!({"failed": result.is_err(), "writes": writes, "logs": logs}))
    }
}
