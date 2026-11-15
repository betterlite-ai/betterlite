mod store;

use std::future::Future;
use std::sync::{Arc, Mutex, PoisonError};

use secrecy::SecretString;
use serde::Deserialize;
use serde_json::{Value, json};
use tracing::subscriber::{DefaultGuard, set_default};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};
use tracing_subscriber::registry;

use self::store::{
    FakeStore, cipher, empty_store, oauth_rows, server_rows, user_credential_rows, user_env_rows,
};
use crate::helpers::{Helper, HelperError, HelperResult};
use betterlite_server::mcp_rotation::rotation::Rotation;

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

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ServerInput {
    pub(crate) server_id: String,
    #[serde(default)]
    pub(crate) credentials: Option<Value>,
    #[serde(default)]
    pub(crate) env_vars: Option<Value>,
    #[serde(default)]
    pub(crate) static_headers: Option<Value>,
    #[serde(default)]
    pub(crate) env: Option<Value>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct OAuthInput {
    pub(crate) server_id: String,
    #[serde(default)]
    pub(crate) credentials: Option<Value>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CredentialInput {
    pub(crate) user_id: String,
    pub(crate) server_id: String,
    pub(crate) credential_b64: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EnvVarInput {
    pub(crate) user_id: String,
    pub(crate) server_id: String,
    pub(crate) values_b64: String,
}

// Internally tagged enums hand each field a ContentDeserializer that cannot serve JsonValue's Box<RawValue>.
#[derive(Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    RotateServers {
        salt_key: String,
        new_master_key: String,
        nonce_hex: String,
        touched_by: String,
        servers: Vec<ServerInput>,
        oauth_clients: Vec<OAuthInput>,
        #[serde(default)]
        fail_at_write: Option<usize>,
    },
    RotateUserCredentials {
        salt_key: String,
        new_master_key: String,
        nonce_hex: String,
        rows: Vec<CredentialInput>,
        #[serde(default)]
        fail_at_write: Option<usize>,
    },
    RotateUserEnvVars {
        salt_key: String,
        new_master_key: String,
        nonce_hex: String,
        rows: Vec<EnvVarInput>,
        #[serde(default)]
        fail_at_write: Option<usize>,
    },
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

fn capture_levels() -> (Levels, DefaultGuard) {
    let levels: Levels = Arc::new(Mutex::new(Vec::new()));
    let layer = LevelCapture {
        levels: Arc::clone(&levels),
    };
    (levels, set_default(registry().with(layer)))
}

fn outcome(failed: bool, writes: &[Value], levels: &Levels) -> Value {
    let logs = levels
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    json!({"failed": failed, "writes": writes, "logs": logs})
}

fn block_on<F: Future>(future: F) -> Result<F::Output, Box<HelperError>> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .map_err(|source| helper_error("RuntimeError", &source.to_string()))?;
    Ok(runtime.block_on(future))
}

const fn keys(input: &Input) -> (&String, &String, &String) {
    match input {
        Input::RotateServers {
            salt_key,
            new_master_key,
            nonce_hex,
            ..
        }
        | Input::RotateUserCredentials {
            salt_key,
            new_master_key,
            nonce_hex,
            ..
        }
        | Input::RotateUserEnvVars {
            salt_key,
            new_master_key,
            nonce_hex,
            ..
        } => (salt_key, new_master_key, nonce_hex),
    }
}

fn prepare_store(
    input: &Input,
    recorder: &Arc<Mutex<Vec<Value>>>,
) -> Result<FakeStore, Box<HelperError>> {
    let mut store = empty_store(Arc::clone(recorder));
    match input {
        Input::RotateServers {
            servers,
            oauth_clients,
            fail_at_write,
            ..
        } => {
            store.servers = server_rows(servers)?;
            store.oauth_clients = oauth_rows(oauth_clients)?;
            store.fail_at = *fail_at_write;
        }
        Input::RotateUserCredentials {
            rows,
            fail_at_write,
            ..
        } => {
            store.user_credentials = user_credential_rows(rows);
            store.fail_at = *fail_at_write;
        }
        Input::RotateUserEnvVars {
            rows,
            fail_at_write,
            ..
        } => {
            store.user_env_vars = user_env_rows(rows);
            store.fail_at = *fail_at_write;
        }
    }
    Ok(store)
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let (salt_key, new_master_key, nonce_hex) = keys(input);
        let cipher = cipher(salt_key, nonce_hex)?;
        let key = SecretString::from(new_master_key.to_owned());
        let recorder = Arc::new(Mutex::new(Vec::new()));
        let store = prepare_store(input, &recorder)?;
        let (levels, guard) = capture_levels();
        let rotation = Rotation {
            store: &store,
            cipher: &cipher,
            new_key: &key,
        };
        let result = match input {
            Input::RotateServers { touched_by, .. } => block_on(rotation.servers(touched_by))?,
            Input::RotateUserCredentials { .. } => block_on(rotation.user_credentials())?,
            Input::RotateUserEnvVars { .. } => block_on(rotation.user_env_vars())?,
        };
        drop(guard);
        let writes = recorder
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        Ok(outcome(result.is_err(), &writes, &levels))
    }
}
