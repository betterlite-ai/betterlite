mod store;

use std::sync::{Arc, Mutex, PoisonError};

use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_server::mcp_rotation::rotation::Rotation;
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::runtime::Builder;
use tracing::subscriber::set_default;
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};
use tracing_subscriber::registry;

use self::store::{cipher, prepare_store};

pub(crate) struct Op;

#[derive(Clone, Copy, Deserialize, PartialEq, Eq)]
pub(crate) enum Function {
    #[serde(rename = "rotate_servers")]
    Servers,
    #[serde(rename = "rotate_user_credentials")]
    UserCredentials,
    #[serde(rename = "rotate_user_env_vars")]
    UserEnvVars,
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
        let runtime = Builder::new_current_thread().build().or_value_error()?;
        let result = match input.function {
            Function::Servers => runtime.block_on(rotation.servers(&input.touched_by)),
            Function::UserCredentials => runtime.block_on(rotation.user_credentials()),
            Function::UserEnvVars => runtime.block_on(rotation.user_env_vars()),
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
