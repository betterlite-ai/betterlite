use std::collections::BTreeSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use betterlite_auth::resource_hints::azure::router_model_in_endpoint;
use betterlite_auth::resource_hints::bedrock::{COUNT_TOKENS_MARKERS, extract_model};
use betterlite_auth::resource_hints::managed::{decode_character_id, decode_video_id};
use betterlite_auth::resource_hints::nvidia::{
    DeploymentHint, MODEL_PREFIX, PROVIDER, model_group_in_path, model_groups,
    router_model_in_endpoint as nim_model_in_endpoint,
};
use serde::Deserialize;
use serde_json::{Value, json};
use tracing::Subscriber;
use tracing::subscriber::with_default;
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::layer::{Context, Layer};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    steps: Vec<Step>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum Step {
    VideoId {
        encoded: String,
    },
    CharacterId {
        encoded: String,
    },
    Azure {
        endpoint: String,
        router_models: Vec<String>,
    },
    NimDeployment {
        custom_llm_provider: Option<String>,
        model: Option<String>,
    },
    NimGroups {
        deployments: Vec<DeploymentInput>,
    },
    NimPath {
        path: String,
        deployments: Vec<DeploymentInput>,
    },
    NimEndpoint {
        endpoint: String,
        router_models: Vec<String>,
    },
    BedrockCount {
        endpoint: String,
    },
    BedrockModel {
        endpoint: String,
    },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DeploymentInput {
    model_name: String,
    custom_llm_provider: Option<String>,
    model: Option<String>,
}

#[derive(Clone, Default)]
struct DebugCounter(Arc<AtomicUsize>);

impl<S: Subscriber> Layer<S> for DebugCounter {
    fn on_event(&self, event: &tracing::Event<'_>, _: Context<'_, S>) {
        if event.metadata().level() == &tracing::Level::DEBUG {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
    }
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        Ok(Value::Array(input.steps.iter().map(run_step).collect()))
    }
}

fn run_step(step: &Step) -> Value {
    match step {
        Step::VideoId { encoded } => managed_value(encoded, ManagedKind::Video),
        Step::CharacterId { encoded } => managed_value(encoded, ManagedKind::Character),
        Step::Azure {
            endpoint,
            router_models,
        } => json!({
            "ok": true,
            "value": {"model": router_model_in_endpoint(endpoint, router_models)},
        }),
        Step::NimDeployment {
            custom_llm_provider,
            model,
        } => json!({
            "ok": true,
            "value": {"is_nim": custom_llm_provider.as_deref() == Some(PROVIDER) || model.as_deref().unwrap_or("").starts_with(MODEL_PREFIX)},
        }),
        Step::NimGroups { deployments } => {
            json!({"ok": true, "value": {"groups": model_groups(&deployment_hints(deployments))}})
        }
        Step::NimPath { path, deployments } => {
            json!({"ok": true, "value": {"model": model_group_in_path(path, &deployment_hints(deployments))}})
        }
        Step::NimEndpoint {
            endpoint,
            router_models,
        } => json!({
            "ok": true,
            "value": {"model": nim_model_in_endpoint(endpoint, &router_models.iter().cloned().collect::<BTreeSet<String>>())},
        }),
        Step::BedrockCount { endpoint } => json!({
            "ok": true,
            "value": {"count_endpoint": COUNT_TOKENS_MARKERS.iter().any(|marker| endpoint.contains(marker))},
        }),
        Step::BedrockModel { endpoint } => extract_model(endpoint).map_or_else(
            |_| json!({"ok": false}),
            |model| json!({"ok": true, "value": {"model": model}}),
        ),
    }
}

#[derive(Clone, Copy)]
enum ManagedKind {
    Video,
    Character,
}

fn managed_value(encoded: &str, kind: ManagedKind) -> Value {
    let counter = DebugCounter::default();
    let subscriber = tracing_subscriber::registry().with(counter.clone());
    let (decoded, debug) = match kind {
        ManagedKind::Video => {
            let decoded = with_default(subscriber, || decode_video_id(encoded));
            let debug = counter.0.load(Ordering::Relaxed);
            (
                json!({
                    "custom_llm_provider": decoded.custom_llm_provider,
                    "model_id": decoded.model_id,
                    "video_id": decoded.video_id,
                }),
                debug,
            )
        }
        ManagedKind::Character => {
            let decoded = with_default(subscriber, || decode_character_id(encoded));
            let debug = counter.0.load(Ordering::Relaxed);
            (
                json!({
                    "custom_llm_provider": decoded.custom_llm_provider,
                    "model_id": decoded.model_id,
                    "character_id": decoded.character_id,
                }),
                debug,
            )
        }
    };
    json!({"ok": true, "value": {"decoded": decoded, "debug": debug}})
}

fn deployment_hints(deployments: &[DeploymentInput]) -> Vec<DeploymentHint<'_>> {
    deployments
        .iter()
        .map(|deployment| DeploymentHint {
            model_name: &deployment.model_name,
            custom_llm_provider: deployment.custom_llm_provider.as_deref(),
            model: deployment.model.as_deref(),
        })
        .collect()
}
