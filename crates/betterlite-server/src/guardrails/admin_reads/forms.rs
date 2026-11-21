use std::sync::Arc;

use axum::Json;
use axum::extract::{Extension, State};
use betterlite_config::loader::EnvironmentLookup;
use betterlite_guardrails::guardrail_registry::rows::GuardrailRows;
use betterlite_pyhost::json_values::{JsonObject, JsonValue};

use super::caller::authenticated;
use super::error::ReadError;
use super::state::ReadState;
use crate::auth::auth_identity::AuthContext;
use crate::guardrails::form_tables_a::tables_a;
use crate::guardrails::form_tables_b::tables_b;
use crate::guardrails::provider_forms::core_tables::core_tables;
use crate::guardrails::provider_forms::project::{project_model, with_friendly_name};
use crate::guardrails::provider_forms::spec::FormModel;

const MAX_RECURSE_DEPTH_VARIABLE: &str = "DEFAULT_MAX_RECURSE_DEPTH";
const DEFAULT_MAX_RECURSE_DEPTH: usize = 100;

pub(super) async fn provider_specific_params<R>(
    caller: Option<Extension<AuthContext>>,
    State(state): State<Arc<ReadState<R>>>,
) -> Result<Json<JsonObject>, ReadError>
where
    R: GuardrailRows,
{
    authenticated(caller)?;
    let max_depth = recursion_depth(state.environment.as_ref())?;
    let mut provider_params = JsonObject::new();
    // the five fixed entries keep LiteLLM's helper, which sets ui_friendly_name only when the model has one
    for (key, model) in core_tables() {
        let mut fields = project_model(&model, max_depth)?;
        with_friendly_name(&model, &mut fields);
        provider_params.insert(key.to_owned(), JsonValue::Object(fields));
    }
    // the registry loop writes ui_friendly_name unconditionally, so a model without one shows null
    for (key, model) in tables_a().into_iter().chain(tables_b()) {
        let mut fields = project_model(&model, max_depth)?;
        fields.insert("ui_friendly_name".to_owned(), friendly_name(&model));
        provider_params.insert(key.to_owned(), JsonValue::Object(fields));
    }
    Ok(Json(provider_params))
}

fn recursion_depth(environment: &dyn EnvironmentLookup) -> Result<usize, ReadError> {
    let Some(text) = environment.lookup(MAX_RECURSE_DEPTH_VARIABLE) else {
        return Ok(DEFAULT_MAX_RECURSE_DEPTH);
    };
    let parsed = betterlite_env::number::<usize>(MAX_RECURSE_DEPTH_VARIABLE, &text)
        .map_err(|source| ReadError::Depth { source })?;
    Ok(parsed.unwrap_or(DEFAULT_MAX_RECURSE_DEPTH))
}

fn friendly_name(model: &FormModel) -> JsonValue {
    let Some(name) = model.ui_friendly_name else {
        return JsonValue::Null;
    };
    JsonValue::String(name.to_owned())
}
