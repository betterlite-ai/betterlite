mod logs;

use std::collections::BTreeMap;

use betterlite_auth::auth_model_lists::credential::{CredentialEntry, CredentialSnapshot};
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_prompts::message_content::Message;
use betterlite_router::catalog_updates::catalog::DeploymentCatalog;
use betterlite_router::deployment_identity::error::InvalidDeployment;
use betterlite_router::deployment_identity::spec::DeploymentSpec;
use betterlite_router::deployment_identity::stable_id::generate_model_id;
use betterlite_router::model_catalog_lookup::alias::AliasMap;
use betterlite_router::model_catalog_lookup::lookup::CatalogLookup;
use betterlite_router::responses_affinity::config::AffinitySettings;
use betterlite_router::responses_affinity::error::{AffinityError, python_type};
use betterlite_router::responses_affinity::filter::AffinityCheck;
use betterlite_router::responses_affinity::history::{FilterRequest, History, MetadataSlot};
use betterlite_router::responses_affinity::identity::deployment_id;
use betterlite_router::routing_groups::index::RoutingGroupIndex;
use betterlite_router::wildcard_catalog::error::PatternCatalogError;
use serde::{Deserialize, Serialize};
use tracing::subscriber::with_default;
use tracing_subscriber::layer::SubscriberExt as _;

use logs::Logs;

const PIN_FLAG: &str = "_encrypted_content_affinity_pinned";
const METADATA_KEY: &str = "litellm_metadata";

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let captured = Logs::default();
        let subscriber = tracing_subscriber::registry().with(captured.clone());
        let answer = with_default(subscriber, || {
            case_answer(input, &captured).or_value_error()
        })?;
        serde_json::to_value(answer).or_value_error()
    }
}

// One helper_case call: the Router the case builds, the healthy pool it hands the filter and the request state it mutates.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub(crate) model: String,
    #[serde(default)]
    pub(crate) model_list: Vec<Row>,
    #[serde(default)]
    pub(crate) healthy_deployments: Vec<Row>,
    #[serde(default)]
    pub(crate) request_kwargs: JsonObject,
    #[serde(default)]
    pub(crate) enable_global_affinity: Option<bool>,
    #[serde(default)]
    pub(crate) model_group_affinity_config: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    pub(crate) credentials: Vec<CredentialRow>,
    #[serde(default)]
    pub(crate) messages: Option<Vec<Message>>,
    #[serde(default)]
    pub(crate) router: Option<bool>,
}

// Router(model_list=...): one deployment as the caller declares it.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Row {
    pub(crate) model_name: String,
    #[serde(default)]
    pub(crate) litellm_params: JsonObject,
    #[serde(default)]
    pub(crate) model_info: JsonObject,
}

// A litellm.credential_list entry: LiteLLM's CredentialItem, minus the credential_info this task never reads.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CredentialRow {
    pub(crate) credential_name: String,
    pub(crate) credential_values: JsonObject,
}

// What a case compares: the narrowed candidates, the history after stripping, the flags, and the diagnostics.
#[derive(Serialize)]
pub(crate) struct Answer {
    pub(crate) ids: Vec<Option<String>>,
    pub(crate) input: Option<Json>,
    pub(crate) messages: Option<Vec<Message>>,
    pub(crate) request_messages: Option<Vec<Message>>,
    pub(crate) metadata: Option<Json>,
    pub(crate) pinned: bool,
    pub(crate) logs: Vec<String>,
    pub(crate) group_affinity_enabled: bool,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum Failure {
    #[error(transparent)]
    Affinity(#[from] AffinityError),
    #[error(transparent)]
    Deployment(#[from] InvalidDeployment),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Pattern(#[from] PatternCatalogError),
}

// The request state the check rewrites in place, as async_filter_deployments reads and mutates it.
struct RequestState {
    input: Option<Json>,
    messages: Option<Vec<Message>>,
    request_messages: Option<Vec<Message>>,
    metadata: Option<Json>,
    pinned: bool,
}

// async_filter_deployments over the case's request state: the constructor's defaults, the filter, then the mutated copies.
pub(crate) fn case_answer(input: &Input, captured: &Logs) -> Result<Answer, Failure> {
    let catalog = catalog_rows(input)?;
    let stored = credential_snapshot(input);
    let healthy = healthy_pool(input);
    let mut state = request_state(input)?;
    let settings = AffinitySettings {
        enable_global_affinity: input.enable_global_affinity.unwrap_or(true),
        model_group_affinity_config: input.model_group_affinity_config.clone(),
    };
    let group_affinity_enabled = settings.has_group_affinity_enabled();
    let aliases = AliasMap::new();
    let groups = RoutingGroupIndex::default();
    let no_inferred = |_: &str| -> Option<String> { None };
    let lookup = CatalogLookup {
        catalog: &catalog,
        model_group_alias: &aliases,
        routing_groups: &groups,
        inferred_provider: &no_inferred,
    };
    let check = AffinityCheck::new(
        settings,
        (input.router != Some(false)).then_some(&lookup),
        &stored,
    );
    let returned = filter_request(input, &mut state, &check, &healthy)?;
    Ok(Answer {
        ids: returned.iter().map(deployment_id).collect(),
        input: state.input,
        messages: state.messages,
        request_messages: state.request_messages,
        metadata: state.metadata,
        pinned: state.pinned,
        logs: captured.messages(),
        group_affinity_enabled,
    })
}

// The filter call itself: the metadata slot and history borrow the state the check rewrites in place.
fn filter_request<'request>(
    input: &'request Input,
    state: &'request mut RequestState,
    check: &AffinityCheck<'_, '_>,
    healthy: &'request [DeploymentSpec],
) -> Result<Vec<DeploymentSpec>, Failure> {
    let metadata = match state.metadata.as_mut() {
        None => MetadataSlot::Absent,
        Some(Json::Object(fields)) => MetadataSlot::Object(fields),
        Some(value) => MetadataSlot::NotAnObject {
            python_type: python_type(value),
        },
    };
    let history = History {
        input: state.input.as_mut(),
        messages: state.messages.as_mut(),
        request_messages: state.request_messages.as_mut(),
        metadata,
        pinned: &mut state.pinned,
    };
    check
        .filter(FilterRequest {
            model: &input.model,
            deployments: healthy,
            history,
        })
        .map_err(Failure::from)
}

// request_kwargs as the check reads it: input, litellm_metadata, messages, and the pin flag it may add.
fn request_state(input: &Input) -> Result<RequestState, Failure> {
    let mut kwargs = input.request_kwargs.clone();
    let request_input = kwargs.shift_remove("input");
    let metadata = kwargs.shift_remove(METADATA_KEY);
    let request_messages = kwargs
        .shift_remove("messages")
        .map(messages_from)
        .transpose()?;
    let pinned = matches!(kwargs.get(PIN_FLAG), Some(Json::Bool(true)));
    Ok(RequestState {
        input: request_input,
        messages: input.messages.clone(),
        request_messages,
        metadata,
        pinned,
    })
}

// The pool the filter narrows: async_filter_deployments' healthy_deployments argument, in the case's order.
fn healthy_pool(input: &Input) -> Vec<DeploymentSpec> {
    if input.healthy_deployments.is_empty() {
        return Vec::new();
    }
    input.healthy_deployments.iter().map(healthy_spec).collect()
}

// The healthy row as the router hands it over: LiteLLM keeps the caller's dict, so no id is invented and a numeric one stays numeric.
fn healthy_spec(row: &Row) -> DeploymentSpec {
    DeploymentSpec {
        model_name: row.model_name.clone(),
        litellm_params: row.litellm_params.clone(),
        model_info: row.model_info.clone(),
        extra: JsonObject::new(),
    }
}

// set_model_list: every row as a deployment under the catalog's indices.
fn catalog_rows(input: &Input) -> Result<DeploymentCatalog, Failure> {
    let mut catalog = DeploymentCatalog::default();
    for row in &input.model_list {
        let spec = spec(row)?;
        catalog.index_patterns(&spec)?;
        catalog.push(spec);
    }
    Ok(catalog)
}

// set_model_list's per-row identity: the declared id, or the stable one it derives from the name and params.
fn spec(row: &Row) -> Result<DeploymentSpec, Failure> {
    let mut model_info = row.model_info.clone();
    if !model_info.contains_key("id") {
        let generated = generate_model_id(&row.model_name, &row.litellm_params);
        model_info.insert("id".to_owned(), Json::Text(generated));
    }
    Ok(DeploymentSpec::new(
        row.model_name.clone(),
        row.litellm_params.clone(),
        model_info,
        JsonObject::new(),
        String::new,
    )?)
}

// litellm.credential_list as the case declares it.
fn credential_snapshot(input: &Input) -> CredentialSnapshot {
    let entries = input
        .credentials
        .iter()
        .map(|row| CredentialEntry {
            credential_name: row.credential_name.clone(),
            credential_values: row.credential_values.clone(),
        })
        .collect();
    CredentialSnapshot::new(entries)
}

// request_kwargs["messages"] holds client message dicts; the filter reads and rewrites them as typed messages.
fn messages_from(value: Json) -> Result<Vec<Message>, Failure> {
    let wire = serde_json::to_value(value)?;
    let decoded: Vec<Message> = serde_json::from_value(wire)?;
    Ok(decoded)
}
