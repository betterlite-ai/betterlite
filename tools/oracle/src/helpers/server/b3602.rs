use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use betterlite_auth::auth_model_lists::credential::{CredentialEntry, CredentialSnapshot};
use betterlite_models::catalog::ModelCatalog;
use betterlite_models::decisions::Json;
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::model_registration::JsonObject;
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::resolution::{ResolutionContext, ResolutionRequest, resolve};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_server::pass_through_credentials::{
    PassThroughDeployment, PassthroughEndpointRouter,
};
use secrecy::ExposeSecret as _;
use serde::Deserialize;
use serde_json::json;

pub(crate) struct Op;

#[derive(Deserialize)]
pub(crate) struct Input {
    deployments: Vec<PassThroughDeployment>,
    credentials: BTreeMap<String, JsonObject>,
    environment: BTreeMap<String, String>,
    #[serde(flatten)]
    call: Call,
}

#[derive(Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
enum Call {
    Credentials {
        provider: String,
        region: Option<String>,
    },
    VertexDeployments {
        model: Option<String>,
    },
    VertexKey {
        project_id: Option<String>,
        location: Option<String>,
        stored: Vec<Stored>,
    },
}

#[derive(Deserialize)]
struct Stored {
    project_id: String,
    location: String,
    credentials: Option<Json>,
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let mut router = router_from(input);
        let result = match &input.call {
            Call::Credentials { provider, region } => {
                json!(
                    router
                        .get_credentials(provider, region.as_deref())
                        .map(|key| key.expose_secret().to_owned())
                )
            }
            Call::VertexDeployments { model } => {
                json!(router.vertex_deployment_credentials(model.as_deref()))
            }
            Call::VertexKey {
                project_id,
                location,
                stored,
            } => {
                for entry in stored {
                    router.add_vertex_credentials(
                        &entry.project_id,
                        &entry.location,
                        entry.credentials.clone(),
                    );
                }
                json!(router.get_vertex_credentials(project_id.as_deref(), location.as_deref()))
            }
        };
        Ok(json!({"result": result}))
    }
}

fn router_from(input: &Input) -> PassthroughEndpointRouter {
    let credentials = CredentialSnapshot::new(
        input
            .credentials
            .iter()
            .map(|(name, values)| CredentialEntry {
                credential_name: name.clone(),
                credential_values: values.clone(),
            })
            .collect(),
    );
    PassthroughEndpointRouter {
        deployments: input.deployments.clone(),
        resolve_provider: Arc::new(resolve_provider),
        credentials,
        environment: Arc::new(input.environment.clone()),
        vertex_by_deployment_key: BTreeMap::new(),
        default_vertex_config: None,
    }
}

fn resolve_provider(model: &str, custom_llm_provider: Option<&str>) -> Option<String> {
    let catalog = ModelCatalog::default();
    let names = BTreeSet::new();
    let environment = BTreeMap::new();
    let context = ResolutionContext {
        catalog: &catalog,
        known_model_names: &names,
        custom_providers: &names,
        environment: &environment,
        registry: JSONProviderRegistry::load(),
        policies: ProviderPolicies::default(),
        globals: GlobalCredentials::default(),
    };
    resolve(
        &ResolutionRequest {
            model,
            custom_llm_provider,
            api_base: None,
            api_key: None,
        },
        &context,
    )
    .ok()
    .map(|resolved| resolved.provider.slug().to_owned())
}
