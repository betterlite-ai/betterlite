use betterlite_secrets::master_key::matches_raw;
use serde_json::Value;

use self::app_crypto_ops::{AppCryptoStep, run_app_crypto};
use self::callback_vars::{CallbackStep, run_callback_vars};
use self::codec::{CodecStep, run_codec};
use self::support::key_source;
use crate::helpers::{Helper, HelperResult};

mod app_crypto_ops;
mod callback_vars;
mod codec;
mod support;
mod tests;

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::Codec {
                salt_key,
                master_key,
                general_settings,
                nonce_hex,
                steps,
            } => {
                let source = key_source(salt_key.as_ref(), master_key.as_ref(), general_settings.as_ref());
                run_codec(&source, nonce_hex, steps)
            }
            Input::AppCrypto { key, steps } => run_app_crypto(key, steps),
            Input::CallbackVars {
                salt_key,
                master_key,
                general_settings,
                nonce_hex,
                extra,
                steps,
            } => {
                let source = key_source(salt_key.as_ref(), master_key.as_ref(), general_settings.as_ref());
                run_callback_vars(&source, nonce_hex, extra, steps)
            }
            Input::MasterKeyMatch {
                presented,
                master_key,
            } => Ok(Value::Bool(matches_raw(
                presented.as_deref(),
                master_key.as_deref(),
            ))),
        }
    }
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    Codec {
        salt_key: Option<String>,
        master_key: Option<String>,
        general_settings: Option<Value>,
        nonce_hex: String,
        steps: Vec<CodecStep>,
    },
    AppCrypto {
        key: String,
        steps: Vec<AppCryptoStep>,
    },
    CallbackVars {
        salt_key: Option<String>,
        master_key: Option<String>,
        general_settings: Option<Value>,
        nonce_hex: String,
        extra: Vec<String>,
        steps: Vec<CallbackStep>,
    },
    MasterKeyMatch {
        presented: Option<String>,
        master_key: Option<String>,
    },
}
