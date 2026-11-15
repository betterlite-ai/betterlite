mod capture;
mod input;
mod store;

use std::sync::{Arc, Mutex};

use secrecy::SecretString;
use serde_json::Value;

use self::capture::{block_on, capture_levels, outcome};
use self::input::Input;
use self::store::{
    FakeStore, cipher, oauth_rows, server_rows, user_credential_rows, user_env_rows, written,
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

fn run_servers(input: &Input) -> HelperResult {
    let Input::RotateServers {
        salt_key,
        new_master_key,
        nonce_hex,
        touched_by,
        servers,
        oauth_clients,
        fail_at_write,
    } = input
    else {
        return Err(helper_error(
            "InvalidInput",
            "the servers function is required",
        ));
    };
    let cipher = cipher(salt_key, nonce_hex)?;
    let key = SecretString::from(new_master_key.clone());
    let recorder = Arc::new(Mutex::new(Vec::new()));
    let store = FakeStore {
        servers: server_rows(servers)?,
        oauth_clients: oauth_rows(oauth_clients)?,
        recorder: Arc::clone(&recorder),
        fail_at: *fail_at_write,
        ..FakeStore::default()
    };
    let (levels, guard) = capture_levels();
    let result = block_on(
        Rotation {
            store: &store,
            cipher: &cipher,
            new_key: &key,
        }
        .servers(touched_by),
    )?;
    drop(guard);
    Ok(outcome(result.is_err(), &written(&recorder), &levels))
}

fn run_user_credentials(input: &Input) -> HelperResult {
    let Input::RotateUserCredentials {
        salt_key,
        new_master_key,
        nonce_hex,
        rows,
        fail_at_write,
    } = input
    else {
        return Err(helper_error(
            "InvalidInput",
            "the user credential function is required",
        ));
    };
    let cipher = cipher(salt_key, nonce_hex)?;
    let key = SecretString::from(new_master_key.clone());
    let recorder = Arc::new(Mutex::new(Vec::new()));
    let store = FakeStore {
        user_credentials: user_credential_rows(rows),
        recorder: Arc::clone(&recorder),
        fail_at: *fail_at_write,
        ..FakeStore::default()
    };
    let (levels, guard) = capture_levels();
    let result = block_on(
        Rotation {
            store: &store,
            cipher: &cipher,
            new_key: &key,
        }
        .user_credentials(),
    )?;
    drop(guard);
    Ok(outcome(result.is_err(), &written(&recorder), &levels))
}

fn run_user_env_vars(input: &Input) -> HelperResult {
    let Input::RotateUserEnvVars {
        salt_key,
        new_master_key,
        nonce_hex,
        rows,
        fail_at_write,
    } = input
    else {
        return Err(helper_error(
            "InvalidInput",
            "the user env var function is required",
        ));
    };
    let cipher = cipher(salt_key, nonce_hex)?;
    let key = SecretString::from(new_master_key.clone());
    let recorder = Arc::new(Mutex::new(Vec::new()));
    let store = FakeStore {
        user_env_vars: user_env_rows(rows),
        recorder: Arc::clone(&recorder),
        fail_at: *fail_at_write,
        ..FakeStore::default()
    };
    let (levels, guard) = capture_levels();
    let result = block_on(
        Rotation {
            store: &store,
            cipher: &cipher,
            new_key: &key,
        }
        .user_env_vars(),
    )?;
    drop(guard);
    Ok(outcome(result.is_err(), &written(&recorder), &levels))
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::RotateServers { .. } => run_servers(input),
            Input::RotateUserCredentials { .. } => run_user_credentials(input),
            Input::RotateUserEnvVars { .. } => run_user_env_vars(input),
        }
    }
}
