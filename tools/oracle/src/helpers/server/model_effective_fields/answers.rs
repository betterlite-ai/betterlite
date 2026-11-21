use std::sync::Mutex;

use betterlite_config::credentials::{CredentialItem, CredentialValues};
use betterlite_config::sections::model_effective_fields::Settings;
use betterlite_db::identity_rows::Payload;
use betterlite_db::stored_cipher::DbCipher;
use betterlite_models::model_registration::JsonObject;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::HelperResult;
use betterlite_secrets::secret_codec::diagnostics::TracingDiagnostics;
use betterlite_secrets::secret_codec::{Algorithm, CipherSettings, OsNonces};
use betterlite_server::credentials::registry::CredentialRegistry;
use betterlite_server::model_effective_fields::credential::{
    CredentialSources, check_credential_name,
};
use betterlite_server::model_effective_fields::effective::{
    effective_complexity_config, effective_model,
};
use betterlite_server::model_effective_fields::ptu::{
    check_ptu_fields_allowed, check_ptu_model_info, explicitly_cleared_ptu_fields,
    merged_ptu_model_info,
};
use betterlite_server::model_effective_fields::rate_limits::check_rate_limits;
use betterlite_server::model_effective_fields::stored::StoredDeployment;
use betterlite_server::model_effective_fields::strategy::{
    RouterMarker, WriteGrant, check_strategy_router_write, member_router_marker,
};
use secrecy::SecretString;
use serde_json::{Value, json};
use tokio::runtime::Builder;

use super::support::{Attribution, FakeCredentials, refusal};
use super::{Call, CredentialCall, Input, MarkerCall, PtuCall};

pub(super) fn answer(input: &Input) -> HelperResult {
    let cipher = DbCipher {
        settings: CipherSettings {
            signing_key: Some(SecretString::from(input.salt_key.clone())),
            algorithm: Algorithm::XSalsa20Poly1305,
        },
        nonces: OsNonces,
        diagnostics: TracingDiagnostics,
    };
    let attribution = Attribution(input.ptu_enabled);
    let mut answers = Vec::with_capacity(input.calls.len());
    for call in &input.calls {
        let answered = match call {
            Call::StrategyRouterWrite(pair) | Call::TuningWrite(pair) => json!({
                "error": refusal(check_strategy_router_write(&cipher, pair.incoming.as_ref(), pair.existing.as_ref())),
            }),
            Call::EffectiveParams(pair) => json!({
                "model": effective_model(&cipher, pair.incoming.as_ref(), pair.existing.as_ref()),
                "complexity_router_config": effective_complexity_config(pair.incoming.as_ref(), pair.existing.as_ref()),
            }),
            Call::MemberMarker(marker_call) => marker_answer(&cipher, marker_call),
            Call::CredentialName(credential_call) => credential_answer(&cipher, credential_call)?,
            Call::RateLimits(rate_call) => {
                let settings = Settings {
                    enforce_rpm_tpm_on_model_add: rate_call.enforced,
                };
                json!({ "error": refusal(check_rate_limits(&rate_call.litellm_params, &settings)) })
            }
            Call::Ptu(ptu_call) => ptu_answer(ptu_call, &attribution),
        };
        answers.push(answered);
    }
    Ok(json!({ "calls": answers }))
}

fn marker_answer(cipher: &DbCipher, call: &MarkerCall) -> Value {
    let existing = StoredDeployment {
        model_id: "model-1".to_owned(),
        model_name: "router".to_owned(),
        litellm_params: call.existing_params.clone(),
        model_info: call.existing_model_info.clone(),
        blocked: false,
    };
    let marker = match member_router_marker(
        cipher,
        call.incoming.as_ref(),
        &existing,
        WriteGrant::Standard,
    ) {
        RouterMarker::Set => Value::Bool(true),
        RouterMarker::Cleared => Value::Bool(false),
        RouterMarker::Unchanged => Value::Null,
    };
    json!({ "marker": marker })
}

fn credential_answer(cipher: &DbCipher, call: &CredentialCall) -> HelperResult {
    let registry = CredentialRegistry::default();
    registry.upsert_credentials(
        call.loaded
            .iter()
            .map(|name| CredentialItem {
                credential_name: name.clone(),
                credential_info: Payload::default(),
                credential_values: CredentialValues::default(),
            })
            .collect(),
    );
    let store = FakeCredentials {
        names: call.stored.clone(),
        lookups: Mutex::new(Vec::new()),
    };
    let sources = CredentialSources {
        cipher,
        registry: &registry,
        store: &store,
    };
    let runtime = Builder::new_current_thread().build().map_err(|error| {
        Box::new(HelperError {
            class: "RuntimeError".to_owned(),
            status: None,
            error_type: None,
            param: None,
            code: None,
            message: error.to_string(),
        })
    })?;
    let outcome = runtime.block_on(check_credential_name(
        call.incoming.as_ref(),
        call.existing.as_ref(),
        &sources,
    ));
    Ok(json!({ "error": refusal(outcome), "lookups": store.lookups() }))
}

fn ptu_answer(call: &PtuCall, attribution: &Attribution) -> Value {
    let mut cleared = explicitly_cleared_ptu_fields(call.incoming.as_ref(), attribution);
    cleared.sort_unstable();
    let merged = merged_ptu_model_info(&call.stored, call.incoming.as_ref(), attribution);
    let sent: JsonObject = call.incoming.clone().unwrap_or_default();
    json!({
        "cleared": cleared,
        "merged": merged,
        "allowed": refusal(check_ptu_fields_allowed(&sent, attribution)),
        "valid": refusal(check_ptu_model_info(&merged)),
    })
}
