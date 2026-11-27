use std::error::Error;

use betterlite_auth::auth_identity::records::Principal;
use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_core::api::file_routing_ids::ManagedResourceAccessChecker;
use betterlite_core::api::vector_store_registry::store::VectorStoreRegistry;
use betterlite_openai::api::vector_store_records::records::vector::LiteLlmManagedVectorStore;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_server::api::vector_file_resolution::error::VectorFileResolutionError;
use betterlite_server::api::vector_file_resolution::file_routing::unified_file_routing;
use betterlite_server::api::vector_file_resolution::model_hint::{
    ModelHintStep, model_hint_precedence,
};
use betterlite_server::api::vector_file_resolution::provider_permissions::provider_and_index_permissions;
use betterlite_server::api::vector_file_resolution::response_ids::{
    FileResponse, restore_file_identifiers,
};
use betterlite_server::api::vector_file_resolution::store_overlay::apply_store_overlay;
use betterlite_server::api::vector_file_resolution::team_fallback::team_model_fallback;
use betterlite_server::api::vector_file_resolution::{Caller, RoutingDeps, StoreDeps};
use serde_json::{Value, json};

use super::input::{
    Call, CatalogScript, Hint, Overlay, Permissions, RequestScript, ResponseScript, Restore,
    Routing,
};
use super::script::{Catalog, Checker, Grants, Prepared, TABLES, caller_principal};

pub(super) async fn answer(call: &Call) -> Result<Value, Box<HelperError>> {
    match call {
        Call::UnifiedFileRouting(routing) => unified_answer(routing).await,
        Call::RestoreFileIdentifiers(restore) => Ok(restore_answer(restore)),
        Call::ModelHintPrecedence(hint) | Call::TeamModelFallback(hint) => hint_answer(hint).await,
        Call::UnifiedStoreAndRegistryOverlay(overlay) => overlay_answer(overlay),
        Call::ProviderAndIndexPermissions(permissions) => permissions_answer(permissions),
    }
}

// The routing call: the data it rewrote, the id it restored, every deployment lookup it made and its error.
async fn unified_answer(routing: &Routing) -> Result<Value, Box<HelperError>> {
    let stage = Stage::new(
        &routing.request,
        routing.catalog.clone(),
        routing.deny.clone(),
    )?;
    let auth =
        UserAPIKeyAuth::construct(AuthInput::Dict(routing.caller.clone())).or_value_error()?;
    let caller = Caller {
        principal: &stage.principal,
        auth: &auth,
    };
    let checker = routing.managed_files.map(|allow| Checker { allow });
    let deps = stage.deps(
        checker
            .as_ref()
            .map(|hook| hook as &dyn ManagedResourceAccessChecker),
        routing.require_managed_files,
    );
    let facts = stage.prepared.facts();
    let mut data = routing.data.clone();
    let outcome = unified_file_routing(&mut data, &routing.file_id, &facts, &caller, &deps).await;
    let (restored, error) = match outcome {
        Ok(restored) => (restored, None),
        Err(found) => (None, Some(error_view(&found))),
    };
    Ok(json!({
        "data": data,
        "restored": restored,
        "lookups": stage.lookups(),
        "error": error,
    }))
}

// The hint and team-fallback call: the data it rewrote, every lookup and authorization it made, and its error.
async fn hint_answer(hint: &Hint) -> Result<Value, Box<HelperError>> {
    let stage = Stage::new(&hint.request, hint.catalog.clone(), hint.deny.clone())?;
    let auth = hint
        .caller
        .as_ref()
        .map(|script| UserAPIKeyAuth::construct(AuthInput::Dict(script.clone())).or_value_error())
        .transpose()?;
    let caller = auth.as_ref().map(|key| Caller {
        principal: &stage.principal,
        auth: key,
    });
    let facts = stage.prepared.facts();
    let deps = stage.deps(None, false);
    let mut data = hint.data.clone();
    let error = match model_hint_precedence(&mut data, &facts, caller.as_ref(), &deps).await {
        Ok(ModelHintStep::FallbackRequired) => {
            team_model_fallback(&mut data, caller.as_ref(), &deps)
                .await
                .err()
                .map(|found| error_view(&found))
        }
        Ok(ModelHintStep::Bypassed | ModelHintStep::CredentialsApplied) => None,
        Err(found) => Some(error_view(&found)),
    };
    Ok(json!({
        "data": data,
        "lookups": stage.lookups(),
        "authorizations": stage.grants.asked_json(),
        "error": error,
    }))
}

// The store overlay call: the data it rewrote and its error.
fn overlay_answer(overlay: &Overlay) -> Result<Value, Box<HelperError>> {
    let managed = overlay
        .managed_store
        .as_ref()
        .map(|script| serde_json::from_value::<LiteLlmManagedVectorStore>(script.clone()))
        .transpose()?;
    let registry = match &overlay.registry {
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
        should_lookup_registry: overlay.should_lookup_registry.unwrap_or(true),
    };
    let mut data = overlay.data.clone();
    let error = apply_store_overlay(&mut data, &overlay.vector_store_id, &deps)
        .err()
        .map(|found| error_view(&found));
    Ok(json!({"data": data, "error": error}))
}

// The provider and index-permission call: the provider it resolved and the answer it failed with.
fn permissions_answer(permissions: &Permissions) -> Result<Value, Box<HelperError>> {
    let prepared = Prepared::new(&permissions.request)?;
    let facts = prepared.facts();
    let principal = caller_principal()?;
    let auth =
        UserAPIKeyAuth::construct(AuthInput::Dict(permissions.caller.clone())).or_value_error()?;
    let caller = Caller {
        principal: &principal,
        auth: &auth,
    };
    match provider_and_index_permissions(
        &permissions.data,
        &permissions.vector_store_id,
        &facts,
        &caller,
        &TABLES,
    ) {
        Ok(provider) => Ok(json!({
            "provider": provider.map(|provider| provider.to_string()),
            "error": Value::Null,
        })),
        Err(found) => Ok(json!({
            "provider": Value::Null,
            "error": {"status": found.status(), "detail": found.to_string()},
        })),
    }
}

// The request, key and scripted router the routing and hint calls share.
struct Stage {
    prepared: Prepared,
    principal: Principal,
    catalog: Option<Catalog>,
    grants: Grants,
}

impl Stage {
    fn new(
        request: &RequestScript,
        catalog: Option<CatalogScript>,
        deny: Vec<String>,
    ) -> Result<Self, Box<HelperError>> {
        Ok(Self {
            prepared: Prepared::new(request)?,
            principal: caller_principal()?,
            catalog: catalog.map(Catalog::new),
            grants: Grants::new(deny),
        })
    }

    fn deps<'stage>(
        &'stage self,
        managed_files: Option<&'stage dyn ManagedResourceAccessChecker>,
        require_managed_files: bool,
    ) -> RoutingDeps<'stage, Catalog, Grants> {
        RoutingDeps {
            catalog: self.catalog.as_ref(),
            grants: &self.grants,
            managed_files,
            require_managed_files,
        }
    }

    fn lookups(&self) -> Value {
        let Some(catalog) = &self.catalog else {
            return Value::Array(Vec::new());
        };
        catalog.lookups_json()
    }
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
