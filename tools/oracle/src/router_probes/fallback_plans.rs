pub mod types;

pub type Input = types::Input;
pub type Output = types::Output;

use std::collections::{BTreeMap, BTreeSet};

use betterlite_config::sections::fallback_plans::Settings;
use betterlite_config::sections::fallback_plans::chains::FallbackChainEntry;
use betterlite_models::catalog::BundledCatalog;
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_router::fallback_plans::attempts::AttemptedTargets;
use betterlite_router::fallback_plans::chains::ProviderNames;
use betterlite_router::fallback_plans::controls::HopRequest;
use betterlite_router::fallback_plans::groups::{
    Bucket, MetadataValues, RequestMetadata, carry_pre_routing_selection,
    clear_pre_routing_selection, fallback_lookup_groups, fallbacks_disabled_for_request,
    get_pre_routing_selection, record_disable_fallbacks, record_pre_routing_selection,
};
use betterlite_router::fallback_plans::plan::{
    DepthOutcome, FallbackBudget, RequestOverrides, ResolvedChain, SelectedChains, advance_depth,
    resolve_chain, select_chains,
};

use self::types::{
    Bucket as BucketInput, BucketOutput, BudgetOutput, ChainOutput, HopOutput, Payload, PlanOutput,
    State,
};
use crate::router_probes::ProbeOutput;
use crate::router_probes::error::OperationError;
use crate::router_probes::random::InjectedRandom;

pub(crate) fn run(
    input: &Input,
    _random: &mut InjectedRandom<'_>,
) -> Result<ProbeOutput, OperationError> {
    let payload = &input.0;
    validate(payload)?;
    let mut state = request_metadata(&payload.state);
    let lookup_groups = lookup_groups(&state, payload);
    let group_refs: Vec<&str> = lookup_groups.iter().map(String::as_str).collect();
    let attempted = AttemptedTargets::from_targets(&payload.attempted);
    let overrides = overrides(payload);
    let configured = Settings {
        fallbacks: payload.config.fallbacks.clone(),
        context_window_fallbacks: payload.config.context_window_fallbacks.clone(),
        content_policy_fallbacks: payload.config.content_policy_fallbacks.clone(),
        max_fallbacks: payload.config.max_fallbacks,
    };
    let selected = select_chains(&configured, &overrides);
    let disabled = fallbacks_disabled_for_request(&state, payload.state.disable_fallbacks);
    apply(&mut state, payload);
    let hop = hop_output(payload, overrides, &state);
    let mut snapshot = request_metadata(&payload.snapshot);
    carry_pre_routing_selection(&state, &mut snapshot);
    let (ordinary, context_window, content_policy) =
        chain_outputs(selected, &group_refs, &attempted);
    Ok(ProbeOutput::FallbackPlans(types::Output(Box::new(
        PlanOutput {
            lookup_groups,
            ordinary,
            context_window,
            content_policy,
            disabled,
            budget: budget_output(selected.budget),
            metadata: bucket_output(&state.metadata),
            litellm_metadata: bucket_output(&state.litellm_metadata),
            carried: get_pre_routing_selection(&snapshot).map(str::to_owned),
            hop,
        },
    ))))
}

// LiteLLM validates the ordinary and content-policy chains only, so a context-window chain keeps bare and multi-key entries
fn validate(payload: &Payload) -> Result<(), OperationError> {
    let chains = [
        &payload.config.fallbacks,
        &payload.config.content_policy_fallbacks,
    ];
    for chain in chains {
        let invalid = chain
            .iter()
            .any(|entry| !matches!(entry, FallbackChainEntry::Group(group) if group.len() == 1));
        if invalid {
            return Err(OperationError::InvalidRequest {
                message: "every configured fallback entry names one model group".to_owned(),
            });
        }
    }
    Ok(())
}

fn lookup_groups(state: &RequestMetadata, payload: &Payload) -> Vec<String> {
    fallback_lookup_groups(state, payload.state.lookup_model_group.as_deref())
        .into_iter()
        .map(str::to_owned)
        .collect()
}

fn overrides(payload: &Payload) -> RequestOverrides<'_> {
    RequestOverrides {
        fallbacks: payload.request.fallbacks.as_deref(),
        context_window_fallbacks: payload.request.context_window_fallbacks.as_deref(),
        content_policy_fallbacks: payload.request.content_policy_fallbacks.as_deref(),
        num_retries: payload.request.num_retries,
        model_group_retry_policy: payload.request.model_group_retry_policy.as_ref(),
        depth: payload.config.depth,
    }
}

// LiteLLM infers a provider from its bundled model_cost, so the router reads the bundled catalog too.
fn chain_outputs(
    selected: SelectedChains<'_>,
    lookup_groups: &[&str],
    attempted: &AttemptedTargets,
) -> (ChainOutput, ChainOutput, ChainOutput) {
    let catalog = BundledCatalog::default().get();
    let known_model_names = BTreeSet::new();
    let environment = BTreeMap::new();
    let registry = JSONProviderRegistry::load();
    let names = ProviderNames {
        catalog: &catalog,
        known_model_names: &known_model_names,
        custom_providers: &known_model_names,
        environment: &environment,
        registry: &registry,
    };
    (
        chain_output(resolve_chain(
            selected.ordinary,
            lookup_groups,
            &names,
            attempted,
        )),
        chain_output(resolve_chain(
            selected.context_window,
            lookup_groups,
            &names,
            attempted,
        )),
        chain_output(resolve_chain(
            selected.content_policy,
            lookup_groups,
            &names,
            attempted,
        )),
    )
}

fn apply(state: &mut RequestMetadata, payload: &Payload) {
    if let Some(selection) = &payload.record_selection {
        clear_pre_routing_selection(state);
        record_pre_routing_selection(state, selection);
    }
    if let Some(disabled) = payload.record_disable {
        record_disable_fallbacks(state, disabled);
    }
}

fn hop_output(
    payload: &Payload,
    overrides: RequestOverrides<'_>,
    live: &RequestMetadata,
) -> HopOutput {
    let model = payload.state.model.as_deref().unwrap_or_default();
    let hop = HopRequest::for_hop(model, overrides, live);
    HopOutput {
        model: hop.model.to_owned(),
        fallbacks: hop.overrides.fallbacks.map(<[FallbackChainEntry]>::to_vec),
        context_window_fallbacks: hop
            .overrides
            .context_window_fallbacks
            .map(<[FallbackChainEntry]>::to_vec),
        content_policy_fallbacks: hop
            .overrides
            .content_policy_fallbacks
            .map(<[FallbackChainEntry]>::to_vec),
        num_retries: hop.overrides.num_retries,
        model_group_retry_policy: hop.overrides.model_group_retry_policy.cloned(),
    }
}

fn chain_output(resolved: Option<ResolvedChain<'_>>) -> ChainOutput {
    match resolved {
        None => ChainOutput {
            resolved: false,
            targets: Vec::new(),
            has_unattempted: false,
        },
        Some(chain) => ChainOutput {
            resolved: true,
            targets: chain
                .targets
                .iter()
                .map(|target| (*target).clone())
                .collect(),
            has_unattempted: chain.has_unattempted,
        },
    }
}

const fn budget_output(budget: FallbackBudget) -> BudgetOutput {
    BudgetOutput {
        depth: budget.depth,
        max_fallbacks: budget.max_fallbacks,
        exhausted: matches!(advance_depth(budget), DepthOutcome::Exhausted),
    }
}

fn bucket_output(bucket: &Bucket) -> BucketOutput {
    BucketOutput {
        pre_routing_selected_model: bucket.values.pre_routing_selected_model.clone(),
        disable_fallbacks: bucket.values.disable_fallbacks,
    }
}

fn request_metadata(state: &State) -> RequestMetadata {
    RequestMetadata {
        metadata: bucket(&state.metadata),
        litellm_metadata: bucket(&state.litellm_metadata),
    }
}

fn bucket(input: &BucketInput) -> Bucket {
    Bucket {
        present: input.present,
        values: MetadataValues {
            pre_routing_selected_model: input.pre_routing_selected_model.clone(),
            model_group: input.model_group.clone(),
            original_model_group: input.original_model_group.clone(),
            disable_fallbacks: input.disable_fallbacks,
        },
    }
}
