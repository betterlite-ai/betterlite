use betterlite_db::resource_rows::model::ProxyModelTable;
use betterlite_db::stored_cipher::DbCipher;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_secrets::secret_codec::diagnostics::TracingDiagnostics;
use betterlite_secrets::secret_codec::{Algorithm, CipherSettings, OsNonces};
use betterlite_server::router_preview_validation::catalog::build_auto_router_catalog;
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    rows: Vec<ProxyModelTable>,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        // no helper case carries a signing key, so every stored string stays as stored on both sides
        let cipher = DbCipher {
            settings: CipherSettings {
                signing_key: None,
                algorithm: Algorithm::XSalsa20Poly1305,
            },
            nonces: OsNonces,
            diagnostics: TracingDiagnostics,
        };
        let catalog = build_auto_router_catalog(&input.rows, &cipher);
        Ok(serde_json::to_value(catalog)?)
    }
}
