use std::error::Error;

use betterlite_core::api::file_routing_ids::ManagedResourceAccessChecker;
use betterlite_core::api::vector_store_registry::store::VectorStoreRegistry;
use betterlite_openai::api::vector_store_records::records::vector::LiteLlmManagedVectorStore;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_server::api::vector_file_resolution::error::VectorFileResolutionError;
use betterlite_server::api::vector_file_resolution::file_routing::unified_file_routing;
use betterlite_server::api::vector_file_resolution::model_hint::{
    ModelHintStep, model_hint_precedence,
};
use betterlite_server::api::vector_file_resolution::provider_permissions::provider_and_index_permissions;
use betterlite_server::api::vector_file_resolution::response_ids::{
    FileResponse, restore_file_identifiers,
};
use betterlite_server::api::vector_file_resolution::store_overlay::unified_store_and_registry_overlay;
use betterlite_server::api::vector_file_resolution::team_fallback::team_model_fallback;
use betterlite_server::api::vector_file_resolution::{Caller, RoutingDeps, StoreDeps};
use serde_json::{Value, json};

use super::input::{Call, Hint, Overlay, Permissions, ResponseScript, Restore, Routing};
use super::script::{Catalog, Checker, Grants, Prepared, TABLES, caller_principal, key_auth};

pub(super) async fn answer(call: &Call) -> Result<Value, Box<HelperError>> {
    match call {
        Call::UnifiedFileRouting(routing) => routing_answer(routing).await,
        Call::RestoreFileIdentifiers(restore) => Ok(restore_answer(restore)),
        Call::ModelHintPrecedence(hint) | Call::TeamModelFallback(hint) => hint_answer(hint).await,
        Call::UnifiedStoreAndRegistryOverlay(overlay) => overlay_answer(overlay),
        Call::ProviderAndIndexPermissions(permissions) => permissions_answer(permissions),
    }
}

// _update_request_data_with_managed_file_id with the router, grants and ownership hook scripted
async fn routing_answer(call: &Routing) -> Result<Value, Box<HelperError>> {
    let prepared = Prepared::new(&call.request)?;
    let facts = prepared.facts();
    let principal = caller_principal()?;
    let auth = key_auth(&call.caller)?;
    let caller = Caller {
        principal: &principal,
        auth: &auth,
    };
    let checker = call.managed_files.map(Checker::new);
    let catalog = call.catalog.clone().map(Catalog::new);
    let grants = Grants::new(call.deny.clone());
    let deps = RoutingDeps {
        catalog: catalog.as_ref(),
        grants: &grants,
        managed_files: checker
            .as_ref()
            .map(|hook| hook as &dyn ManagedResourceAccessChecker),
        require_managed_files: call.require_managed_files,
    };
    let mut data = call.data.clone();
    let outcome = unified_file_routing(&mut data, &call.file_id, &facts, &caller, &deps).await;
    let (restored, error) = match outcome {
        Ok(restored) => (restored, None),
        Err(found) => (None, Some(error_view(&found))),
    };
    Ok(json!({
        "data": data,
        "restored": restored,
        "lookups": lookups(catalog.as_ref()),
        "error": error,
    }))
}

// _replace_file_id_in_response over the response shape the case names
fn restore_answer(call: &Restore) -> Value {
    match &call.response {
        ResponseScript::Null => json!({"response": null}),
        ResponseScript::Dict { fields } => {
            let mut document = fields.clone();
            restore_file_identifiers(
                Some(FileResponse::Fields(&mut document)),
                &call.original_file_id,
            );
            json!({"response": document})
        }
        ResponseScript::Object { id, file_id } => {
            let mut id_text = id.clone();
            let mut file_id_text = file_id.clone();
            let response = match (id_text.as_mut(), file_id_text.as_mut()) {
                (Some(identifier), Some(provider_file)) => FileResponse::Both {
                    id: identifier,
                    file_id: provider_file,
                },
                (Some(identifier), None) => FileResponse::Identifier(identifier),
                (None, Some(provider_file)) => FileResponse::FileIdentifier(provider_file),
                (None, None) => return json!({"response": {}}),
            };
            restore_file_identifiers(Some(response), &call.original_file_id);
            let mut restored = serde_json::Map::new();
            if let Some(text) = id_text {
                restored.insert(String::from("id"), Value::String(text));
            }
            if let Some(text) = file_id_text {
                restored.insert(String::from("file_id"), Value::String(text));
            }
            json!({"response": restored})
        }
    }
}

// _update_request_data_with_model_routing_hint, its hint half and then the team fallback when a caller is scripted
async fn hint_answer(call: &Hint) -> Result<Value, Box<HelperError>> {
    let prepared = Prepared::new(&call.request)?;
    let facts = prepared.facts();
    let principal = caller_principal()?;
    let auth = call.caller.as_ref().map(key_auth).transpose()?;
    let caller = auth.as_ref().map(|key| Caller {
        principal: &principal,
        auth: key,
    });
    let catalog = call.catalog.clone().map(Catalog::new);
    let grants = Grants::new(call.deny.clone());
    let deps = RoutingDeps {
        catalog: catalog.as_ref(),
        grants: &grants,
        managed_files: None,
        require_managed_files: false,
    };
    let mut data = call.data.clone();
    let mut error = None;
    match model_hint_precedence(&mut data, &facts, caller.as_ref(), &deps).await {
        Ok(ModelHintStep::FallbackRequired) => {
            if let Err(found) = team_model_fallback(&mut data, caller.as_ref(), &deps).await {
                error = Some(error_view(&found));
            }
        }
        Ok(ModelHintStep::Bypassed | ModelHintStep::CredentialsApplied) => {}
        Err(found) => error = Some(error_view(&found)),
    }
    Ok(json!({
        "data": data,
        "lookups": lookups(catalog.as_ref()),
        "authorizations": grants.asked_json(),
        "error": error,
    }))
}

// _update_request_data_with_litellm_managed_vector_store_registry over the store the route resolved and the registry script
fn overlay_answer(call: &Overlay) -> Result<Value, Box<HelperError>> {
    let managed = call
        .managed_store
        .as_ref()
        .map(|script| serde_json::from_value::<LiteLlmManagedVectorStore>(script.clone()))
        .transpose()?;
    let registry = match &call.registry {
        Some(stores) => {
            let mut built = VectorStoreRegistry::default();
            for script in stores {
                built
                    .stores
                    .push(serde_json::from_value::<LiteLlmManagedVectorStore>(
                        script.clone(),
                    )?);
            }
            Some(built)
        }
        None => None,
    };
    let deps = StoreDeps {
        managed_store: managed.as_ref(),
        registry: registry.as_ref(),
        should_lookup_registry: call.should_lookup_registry.unwrap_or(true),
    };
    let mut data = call.data.clone();
    let error = unified_store_and_registry_overlay(&mut data, &call.vector_store_id, &deps)
        .err()
        .map(|found| error_view(&found));
    Ok(json!({"data": data, "error": error}))
}

// _resolve_provider then _maybe_check_permissions, B1061 reading the scripted key and the real endpoint tables
fn permissions_answer(call: &Permissions) -> Result<Value, Box<HelperError>> {
    let prepared = Prepared::new(&call.request)?;
    let facts = prepared.facts();
    let principal = caller_principal()?;
    let auth = key_auth(&call.caller)?;
    let caller = Caller {
        principal: &principal,
        auth: &auth,
    };
    let outcome =
        provider_and_index_permissions(&call.data, &call.vector_store_id, &facts, &caller, &TABLES);
    let (provider, error) = match outcome {
        Ok(provider) => (provider.map(|provider| provider.to_string()), None),
        Err(found) => (
            None,
            Some(json!({"status": found.status(), "detail": found.to_string()})),
        ),
    };
    Ok(json!({"provider": provider, "error": error}))
}

fn lookups(catalog: Option<&Catalog>) -> Value {
    catalog.map_or(Value::Array(Vec::new()), Catalog::lookups_json)
}

// the status and detail the route would answer with, as LiteLLM's HTTPException carries them
fn error_view<CatalogError, GrantError>(
    error: &VectorFileResolutionError<CatalogError, GrantError>,
) -> Value
where
    CatalogError: Error + 'static,
    GrantError: Error + 'static,
{
    match error {
        VectorFileResolutionError::ManagedId(source) => {
            json!({"status": source.status(), "detail": source.to_string()})
        }
        VectorFileResolutionError::Authorization(source) => {
            json!({"status": 403_u16, "detail": source.to_string()})
        }
        VectorFileResolutionError::Lookup(source) => {
            json!({"status": source.status(), "detail": source.detail_error()})
        }
        VectorFileResolutionError::Catalog { .. }
        | VectorFileResolutionError::Caller(_)
        | VectorFileResolutionError::Store(_)
        | VectorFileResolutionError::StoreParams => {
            json!({"status": Value::Null, "detail": error.to_string()})
        }
    }
}
