use std::collections::BTreeMap;

use betterlite_auth::sso_resolution::{
    SSO_DESCRIPTORS, resolve_sso_config, sso_field_env_vars, sso_secret_fields,
};
use betterlite_db::stored_cipher::DbCipher;
use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_secrets::secret_codec::diagnostics::TracingDiagnostics;
use betterlite_secrets::secret_codec::{Algorithm, CipherSettings, OsNonces};
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    salt_key: String,
    db: Option<BTreeMap<String, Value>>,
    env: BTreeMap<String, String>,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let cipher = DbCipher {
            settings: CipherSettings {
                signing_key: Some(SecretString::from(input.salt_key.clone())),
                algorithm: Algorithm::XSalsa20Poly1305,
            },
            nonces: OsNonces,
            diagnostics: TracingDiagnostics,
        };
        let environment = input
            .env
            .iter()
            .map(|(name, value)| (name.clone(), SecretString::from(value.clone())))
            .collect::<BTreeMap<_, _>>();
        let resolved =
            resolve_sso_config(input.db.as_ref(), &environment, &cipher).or_value_error()?;
        let descriptors = SSO_DESCRIPTORS
            .iter()
            .map(|descriptor| {
                json!({
                    "field_name": descriptor.field_name,
                    "db_key": descriptor.db_key,
                    "env_var": descriptor.env_var,
                    "is_secret": descriptor.is_secret,
                    "default": descriptor.default,
                })
            })
            .collect::<Vec<Value>>();
        Ok(json!({
            "config": resolved.config,
            "provenance": resolved.provenance,
            "descriptors": descriptors,
            "secret_fields": sso_secret_fields(),
            "field_env_vars": sso_field_env_vars(),
        }))
    }
}
