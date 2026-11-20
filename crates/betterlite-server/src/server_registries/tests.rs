#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, PoisonError};

use betterlite_auth::auth_model_lists::credential::CredentialSnapshot;
use betterlite_config::loader::EnvironmentLookup;
use betterlite_config::sections::vertex_credential_sources::credential_source::ConfiguredCredentials;
use betterlite_vertex::credential_sources::sdk::{CredentialClock, SystemClock};
use betterlite_vertex::token_cache::error::CredentialCacheError;
use betterlite_vertex::token_cache::executor::{CredentialExecutor, LoadedCredential};
use betterlite_vertex::token_cache::key::CredentialKey;
use betterlite_vertex::token_cache::record::{
    CacheRecord, CredentialSnapshot as VertexCredentialSnapshot,
};
use secrecy::SecretString;

use super::PassThroughState;
use crate::pass_through_credentials::ProviderResolver;
use crate::passthrough::route_registration::{PassthroughParams, RouteSpec, add_exact_path_route};

struct NoCredentials;

impl CredentialExecutor for NoCredentials {
    fn load<'borrow>(
        &'borrow self,
        _credentials: Option<&'borrow ConfiguredCredentials>,
        _project: Option<&'borrow str>,
    ) -> Pin<
        Box<dyn Future<Output = Result<LoadedCredential, CredentialCacheError>> + Send + 'borrow>,
    > {
        Box::pin(async { Err(CredentialCacheError::MissingCredentials) })
    }

    fn refresh<'borrow>(
        &'borrow self,
        _credential: &'borrow VertexCredentialSnapshot,
    ) -> Pin<
        Box<
            dyn Future<Output = Result<VertexCredentialSnapshot, CredentialCacheError>>
                + Send
                + 'borrow,
        >,
    > {
        Box::pin(async {
            Ok(VertexCredentialSnapshot {
                token: None,
                state: None,
                expires_at: None,
                expired: None,
                valid: None,
                quota_project_id: None,
                project_id: None,
            })
        })
    }
}

fn no_provider(_model: &str, _provider: Option<&str>) -> Option<String> {
    None
}

fn empty_state() -> PassThroughState {
    let environment: BTreeMap<String, String> = BTreeMap::new();
    let lookup: Arc<dyn EnvironmentLookup + Send + Sync> = Arc::new(environment);
    let clock: Arc<dyn CredentialClock> = Arc::new(SystemClock);
    let executor: Arc<dyn CredentialExecutor> = Arc::new(NoCredentials);
    let resolver: ProviderResolver = Arc::new(no_provider);
    PassThroughState::new(
        clock,
        executor,
        resolver,
        CredentialSnapshot::default(),
        lookup,
    )
}

#[test]
fn pass_new_state_starts_empty() {
    let state = empty_state();
    let router = state
        .credentials
        .read()
        .unwrap_or_else(PoisonError::into_inner);
    assert!(router.deployments.is_empty());
    assert!(router.vertex_by_deployment_key.is_empty());
    assert!(router.get_vertex_credentials(None, None).is_none());
    assert!(router.vertex_deployment_credentials(None).is_none());
    drop(router);
    let tokens = &state.vertex_tokens;
    assert!(format!("{tokens:?}").contains("records: 0"));
    assert!(tokens.leased_identities().is_empty());
    let pass_through = state.routes.lock().unwrap_or_else(PoisonError::into_inner);
    assert!(pass_through.table.order().is_empty());
    assert!(pass_through.endpoints.is_empty());
    assert!(pass_through.registry.registrations.is_empty());
    drop(pass_through);
}

#[test]
fn pass_route_written_through_one_clone_is_seen_through_another() {
    let state = empty_state();
    let writer = Arc::clone(&state.routes);
    let reader = Arc::clone(&state.routes);
    let methods = vec![String::from("POST")];
    let params = PassthroughParams {
        target: String::from("http://127.0.0.1:9/echo"),
        custom_headers: None,
        forward_headers: None,
        merge_query_params: None,
        default_query_params: None,
        cost_per_request: None,
        guardrails: None,
        timeout: None,
        config_file_path: None,
    };
    let spec = RouteSpec {
        endpoint_id: "echo",
        path: "/echo",
        methods: &methods,
        auth: true,
        params: &params,
    };
    add_exact_path_route(&writer, &spec, Some("/config.yaml"));
    let routes = reader.lock().unwrap_or_else(PoisonError::into_inner);
    let installed: BTreeSet<String> = methods.iter().cloned().collect();
    assert!(routes.table.is_path_registered("/echo", &installed));
    assert_eq!(routes.endpoints.len(), 1);
    let stored = routes.endpoints.values().next().unwrap();
    assert_eq!(stored.target, "http://127.0.0.1:9/echo");
    assert_eq!(stored.config_file_path.as_deref(), Some("/config.yaml"));
    assert_eq!(routes.registry.registrations.len(), 1);
}

#[test]
fn pass_vertex_default_config_written_through_one_clone_is_seen_through_another() {
    let state = empty_state();
    let writer = Arc::clone(&state.credentials);
    let reader = Arc::clone(&state.credentials);
    writer
        .write()
        .unwrap_or_else(PoisonError::into_inner)
        .set_default_vertex_config(None)
        .unwrap();
    let router = reader.read().unwrap_or_else(PoisonError::into_inner);
    assert!(router.get_vertex_credentials(None, None).is_some());
    drop(router);
}

#[test]
fn pass_vertex_tokens_written_through_one_clone_are_seen_through_another() {
    let state = empty_state();
    let key = CredentialKey::new(None, None);
    let cached = Arc::new(VertexCredentialSnapshot {
        token: Some(SecretString::from("token")),
        state: None,
        expires_at: None,
        expired: None,
        valid: None,
        quota_project_id: None,
        project_id: None,
    });
    let writer = Arc::clone(&state.vertex_tokens);
    let reader = Arc::clone(&state.vertex_tokens);
    writer.store_record(
        key.clone(),
        CacheRecord::Paired {
            credential: cached,
            project: None,
        },
    );
    assert!(reader.cached_record(&key).is_some());
}
