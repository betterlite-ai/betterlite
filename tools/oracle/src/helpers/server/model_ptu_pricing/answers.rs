use std::collections::BTreeMap;
use std::convert::Infallible;
use std::future::{Future, ready};
use std::sync::atomic::{AtomicUsize, Ordering};

use betterlite_db::stored_cipher::DbCipher;
use betterlite_models::decisions::Json;
use betterlite_models::model_catalog_keys::ModelCostKeys;
use betterlite_models::model_registration::JsonObject;
use betterlite_models::request_pricing::fields::echoed_cost_map_fields;
use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::HelperResult;
use betterlite_router::deployment_model_info::metadata::ModelMetadataSource;
use betterlite_secrets::secret_codec::diagnostics::TracingDiagnostics;
use betterlite_secrets::secret_codec::{Algorithm, CipherSettings, OsNonces};
use betterlite_server::jobs::ptu::attribution::PtuAttribution;
use betterlite_server::model_effective_fields::error::Refusal;
use betterlite_server::model_ptu_pricing::DeploymentBlobs;
use betterlite_server::model_ptu_pricing::catalog::{
    LoadedCatalog, cost_map_entry, loaded_catalog_entry,
};
use betterlite_server::model_ptu_pricing::delta::{PtuPatch, ptu_pricing_delta};
use betterlite_server::model_ptu_pricing::error::PtuPricingError;
use betterlite_server::model_ptu_pricing::prices::{
    is_nonzero_price, is_nonzero_rate, is_zero_price,
};
use betterlite_server::model_ptu_pricing::zeroed::{
    check_ptu_unpriced, ptu_priced_deployment, ptu_zeroed_pricing, write_ptu_pricing,
};
use secrecy::SecretString;
use serde_json::{Value, json};
use tokio::runtime::{Builder, Runtime};

use super::{Call, CatalogCall, DeltaCall, DeploymentCall, Input, PricesCall, ZeroedCall};

struct Attribution(bool);

impl PtuAttribution for Attribution {
    fn enabled(&self) -> bool {
        self.0
    }
}

// a cost map table from the case: get_model_info finds an entry by its name, and LiteLLM's raise for an unmapped one is suppressed like a miss
struct Table {
    entries: BTreeMap<String, JsonObject>,
    reads: AtomicUsize,
}

impl ModelMetadataSource for Table {
    type Error = Infallible;

    fn get_model_info(
        &self,
        model: &str,
        _keys: &mut ModelCostKeys,
    ) -> impl Future<Output = Result<Option<JsonObject>, Infallible>> + Send {
        ready(Ok(self.entries.get(model).cloned()))
    }
}

impl LoadedCatalog for Table {
    fn entry(&self, key: &str) -> Option<JsonObject> {
        self.reads.fetch_add(1, Ordering::Relaxed);
        self.entries.get(key).cloned()
    }
}

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
    let runtime = Builder::new_current_thread().build().or_value_error()?;
    let mut answers = Vec::with_capacity(input.calls.len());
    for call in &input.calls {
        let answered = match call {
            Call::Prices(prices_call) => prices_answer(prices_call),
            Call::Priced(priced_call) => {
                let outcome = check_ptu_unpriced(
                    &priced_call.model_info,
                    &priced_call.supplied,
                    &attribution,
                );
                json!({ "error": outcome.err().as_ref().map(refusal) })
            }
            Call::Zeroed(zeroed_call) => zeroed_answer(zeroed_call, &attribution)?,
            Call::Delta(delta_call) => delta_answer(delta_call, &attribution)?,
            Call::PricedDeployment(deployment_call) => {
                deployment_answer(deployment_call, &attribution)
            }
            Call::Catalog(catalog_call) => catalog_answer(&cipher, &runtime, catalog_call),
        };
        answers.push(answered);
    }
    Ok(json!({ "calls": answers }))
}

fn prices_answer(call: &PricesCall) -> Value {
    let found: Vec<[bool; 3]> = call
        .values
        .iter()
        .map(|value| {
            [
                is_nonzero_rate(value),
                is_nonzero_price(value),
                is_zero_price(value),
            ]
        })
        .collect();
    json!({ "answers": found })
}

// a bare HTTPException carries its status and neither type, param nor code
fn refusal(error: &PtuPricingError) -> Value {
    let refused = Refusal::from(error);
    json!({
        "status": refused.status,
        "type": refused.error_type.map(|kind| kind.to_string()),
        "param": refused.param,
        "code": Value::Null,
    })
}

fn zeroed_answer(call: &ZeroedCall, attribution: &Attribution) -> HelperResult {
    let blobs = DeploymentBlobs {
        model_info: &call.model_info,
        litellm_params: &call.litellm_params,
    };
    let pricing = match ptu_zeroed_pricing(blobs, &call.supplied, attribution) {
        Ok(pricing) => pricing,
        Err(error) => return Ok(json!({ "pricing": null, "error": refusal(&error) })),
    };
    let mut written = JsonObject::new();
    write_ptu_pricing(&pricing, &mut written).or_value_error()?;
    Ok(json!({ "pricing": written, "error": null }))
}

fn delta_answer(call: &DeltaCall, attribution: &Attribution) -> HelperResult {
    let blobs = DeploymentBlobs {
        model_info: &call.model_info,
        litellm_params: &call.litellm_params,
    };
    let patch = PtuPatch {
        litellm_params: call.patch.litellm_params.as_ref(),
        model_info: call.patch.model_info.as_ref(),
    };
    let delta = match ptu_pricing_delta(&call.stored_model_info, blobs, patch, attribution) {
        Ok(delta) => delta,
        Err(error) => {
            return Ok(json!({ "written": null, "released": null, "error": refusal(&error) }));
        }
    };
    let mut written = JsonObject::new();
    write_ptu_pricing(&delta.written, &mut written).or_value_error()?;
    Ok(json!({ "written": written, "released": delta.released, "error": null }))
}

fn deployment_answer(call: &DeploymentCall, attribution: &Attribution) -> Value {
    let mut litellm_params = call.litellm_params.clone();
    let mut model_info = call.model_info.clone();
    match ptu_priced_deployment(&mut litellm_params, &mut model_info, attribution) {
        Ok(()) => {
            json!({ "litellm_params": litellm_params, "model_info": model_info, "error": null })
        }
        Err(error) => {
            json!({ "litellm_params": null, "model_info": null, "error": refusal(&error) })
        }
    }
}

fn catalog_answer(cipher: &DbCipher, runtime: &Runtime, call: &CatalogCall) -> Value {
    let cost_map = Table {
        entries: call.cost_map.clone(),
        reads: AtomicUsize::new(0),
    };
    let catalog = Table {
        entries: call.catalog.clone(),
        reads: AtomicUsize::new(0),
    };
    let mut stored_params = JsonObject::new();
    stored_params.insert("model".to_owned(), Json::Text(call.model.clone()));
    let mut keys = ModelCostKeys::default();
    let cost_entry = runtime.block_on(cost_map_entry(
        &cost_map,
        &mut keys,
        cipher,
        &stored_params,
        &call.incoming,
    ));
    let catalog_entry = loaded_catalog_entry(&call.incoming, &catalog);
    json!({
        "echoed": echoed_cost_map_fields(&call.incoming, &[&cost_entry, &catalog_entry]),
        "catalog_entry": catalog_entry,
        "catalog_loaded": catalog.reads.load(Ordering::Relaxed) > 0,
    })
}
