use betterlite_openai::watsonx_auth::credentials::get_api_params;
use betterlite_openai::watsonx_auth::environment::WatsonxEnvironment;
use betterlite_openai::watsonx_auth::payload::prepare_payload;
use betterlite_openai::watsonx_auth::types::WatsonxParams;

use super::error::WatsonxRerankError;
use crate::api::rerank_contracts::Params;

pub fn transform_rerank_request(
    model: &str,
    params: Params,
    environment: &WatsonxEnvironment,
) -> Result<Params, WatsonxRerankError> {
    let mut wire: WatsonxParams = serde_json::from_value(serde_json::to_value(&params)?)?;
    let api_params = get_api_params(&mut wire, Some(model), environment)?;
    let identity = prepare_payload(model, &api_params);
    let mut merged: Params = serde_json::from_value(serde_json::to_value(&wire)?)?;
    merged.extend(serde_json::from_value::<Params>(serde_json::to_value(
        identity,
    )?)?);
    Ok(merged)
}
