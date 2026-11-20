use std::collections::{BTreeMap, BTreeSet};
use std::future::{Future, ready};
use std::sync::Arc;

use betterlite_core::cost::completion::lookup::{CatalogLookup, LookupSettings};
use betterlite_models::catalog::{BundledCatalog, ModelCatalog, UnknownFields};
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::price_lookup::{PriceLookup, PriceRecord, PricedModel};
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::savings_evidence::billed_at::BilledAt;
use betterlite_models::savings_evidence::dynamic::MappingOrScalar;
use betterlite_models::savings_evidence::metadata::AutorouterEstimate;
use betterlite_models::savings_evidence::snapshot::{BaselineCostSnapshot, CostBreakdown};
use betterlite_models::savings_evidence::usage::RawUsageObject;
use betterlite_models::savings_pricing::autorouter::{
    AutoRouterDecision, AutoRouterProvenance, AutoRouterSavingsInput, compute_autorouter_savings,
};
use betterlite_models::savings_pricing::compare::{
    ComparisonInput, ComparisonProvenance, price_baseline_comparison,
};
use betterlite_models::savings_pricing::context::{ModelIdentity, SavingsPricingContext};
use betterlite_models::savings_pricing::spend::{SavingsSpendInput, compute_savings_spend};
use betterlite_models::savings_pricing::usage::{RequestUsage, request_usage};
use betterlite_models::tiered_pricing::Scalar;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_server::cost_service::name_normalizers;
use chrono::DateTime;
use jiff::tz::TimeZoneDatabase;

pub(crate) struct Op;

// Every operation's fields sit beside the tag; a case sends only the ones its operation reads.
#[derive(serde::Deserialize)]
pub(crate) struct Input {
    operation: Operation,
    usage: Option<UsageInput>,
    baseline_model: Option<String>,
    selected_model: Option<String>,
    selected_provider: Option<String>,
    baseline_usage: Option<UsageInput>,
    selected_info: Option<PriceRecord>,
    baseline_info: Option<PriceRecord>,
    cost_breakdown: Option<CostBreakdown>,
    baseline_deployment_id: Option<String>,
    selected_deployment_id: Option<String>,
    baseline_provenance: Option<AutoRouterProvenance>,
    snapshot: Option<BaselineCostSnapshot>,
    provenance: Option<ComparisonProvenance>,
    model: Option<String>,
    custom_llm_provider: Option<String>,
    #[serde(default)]
    compression_saved_tokens: i64,
    #[serde(default)]
    gateway_injected_cache: bool,
    routing_decision: Option<AutoRouterDecision>,
    model_id: Option<String>,
    recorded_autorouter_savings: Option<Scalar>,
    recorded_estimate: Option<MappingOrScalar<AutorouterEstimate>>,
}

#[derive(Clone, Copy, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
enum Operation {
    ComputeAutorouterSavings,
    PriceBaselineComparison,
    ComputeSavingsSpend,
}

// The spend-log usage shape the cases send; B1693's parser reads the same fields.
#[derive(Clone, Debug, Default, serde::Deserialize, serde::Serialize)]
#[serde(default)]
struct UsageInput {
    prompt_tokens: Option<i64>,
    completion_tokens: Option<i64>,
    total_tokens: Option<i64>,
    cache_read_input_tokens: Option<i64>,
    cache_creation_input_tokens: Option<i64>,
    prompt_tokens_details: Option<DetailsInput>,
}

#[derive(Clone, Debug, Default, serde::Deserialize, serde::Serialize)]
#[serde(default)]
struct DetailsInput {
    cached_tokens: Option<i64>,
}

// The helper's own resolver: a provider prefix names the provider, as get_llm_provider reads these cases.
struct CaseContext {
    lookup: CatalogLookup,
}

impl SavingsPricingContext for CaseContext {
    fn resolve_model(
        &self,
        model: &str,
        custom_llm_provider: Option<&str>,
    ) -> Option<ModelIdentity> {
        let (provider, name) = match model.split_once('/') {
            Some((provider, name)) => (provider.to_owned(), name),
            None => (custom_llm_provider?.to_owned(), model),
        };
        (!name.is_empty()).then(|| ModelIdentity {
            model: name.to_owned(),
            provider,
        })
    }

    // No deployment registry exists in the oracle: every case prices from supplied or public rates.
    fn deployment_price(
        &self,
        _deployment_id: Option<&str>,
        _model: &str,
    ) -> impl Future<Output = Option<PricedModel>> + Send {
        ready(None)
    }

    fn public_lookup(&self) -> &dyn PriceLookup {
        &self.lookup
    }
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let context = case_context()?;
        let database = TimeZoneDatabase::none();
        match input.operation {
            Operation::ComputeAutorouterSavings => {
                let Some(usage) = input.usage.as_ref().and_then(usage_input) else {
                    return Err(helper_error("InvalidInput", "usage could not be read"));
                };
                let baseline_usage = input.baseline_usage.as_ref().and_then(usage_input);
                let selected_info = priced(input.selected_info.as_ref());
                let baseline_info = priced(input.baseline_info.as_ref());
                let result = compute_autorouter_savings(
                    &context,
                    &AutoRouterSavingsInput {
                        baseline_model: input.baseline_model.as_deref(),
                        selected_model: input.selected_model.as_deref(),
                        selected_provider: input.selected_provider.as_deref(),
                        usage: &usage,
                        selected_info: selected_info.as_ref(),
                        baseline_info: baseline_info.as_ref(),
                        cost_breakdown: input.cost_breakdown.as_ref(),
                        baseline_deployment_id: input.baseline_deployment_id.as_deref(),
                        selected_deployment_id: input.selected_deployment_id.as_deref(),
                        baseline_usage: baseline_usage.as_ref(),
                        baseline_provenance: input.baseline_provenance,
                        moment: DateTime::UNIX_EPOCH,
                        database: &database,
                    },
                );
                Ok(serde_json::json!({
                    "savings": result.map(|value| value.normalize().to_string()),
                }))
            }
            Operation::PriceBaselineComparison => {
                let Some(snapshot) = input.snapshot.as_ref() else {
                    return Err(helper_error("InvalidInput", "snapshot is required"));
                };
                let baseline_usage = input.baseline_usage.as_ref().and_then(usage_input);
                let result = price_baseline_comparison(
                    &context,
                    &ComparisonInput {
                        snapshot,
                        baseline_usage: baseline_usage.as_ref(),
                        provenance: input.provenance,
                        moment: DateTime::UNIX_EPOCH,
                        database: &database,
                    },
                );
                Ok(result.map_or_else(
                    || serde_json::json!({ "actual": null, "baseline": null }),
                    |costs| {
                        serde_json::json!({
                            "actual": costs.actual.normalize().to_string(),
                            "baseline": costs.baseline.normalize().to_string(),
                        })
                    },
                ))
            }
            Operation::ComputeSavingsSpend => {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .map_err(|error| helper_error("RuntimeError", &error.to_string()))?;
                let usage = input.usage.as_ref().and_then(usage_input);
                let billed_at = BilledAt::Missing;
                let request = SavingsSpendInput {
                    model: input.model.as_deref(),
                    custom_llm_provider: input.custom_llm_provider.as_deref(),
                    compression_saved_tokens: input.compression_saved_tokens,
                    gateway_injected_cache: input.gateway_injected_cache,
                    routing_decision: input.routing_decision.as_ref(),
                    usage: usage.as_ref(),
                    model_id: input.model_id.as_deref(),
                    cost_breakdown: input.cost_breakdown.as_ref(),
                    recorded_autorouter_savings: input.recorded_autorouter_savings.as_ref(),
                    recorded_estimate: input.recorded_estimate.as_ref(),
                    billed_at: &billed_at,
                    fallback_moment: DateTime::UNIX_EPOCH,
                    database: &database,
                };
                let spend = runtime.block_on(compute_savings_spend(&context, &request));
                Ok(serde_json::json!({
                    "compression": spend.compression.normalize().to_string(),
                    "prompt_caching": spend.prompt_caching.normalize().to_string(),
                    "autorouter": spend.autorouter.normalize().to_string(),
                    "gateway_injected_caching": spend.gateway_injected_caching.normalize().to_string(),
                }))
            }
        }
    }
}

// The public lookup every case prices from is the same bundled catalog the proxy ships.
fn case_context() -> Result<CaseContext, Box<HelperError>> {
    let bundled = BundledCatalog::default();
    let asset =
        (bundled.asset)().map_err(|error| helper_error("CatalogError", &error.to_string()))?;
    let catalog = ModelCatalog::parse(&asset, UnknownFields::Warn)
        .map_err(|error| helper_error("CatalogError", &error.to_string()))?;
    let settings = LookupSettings {
        known_model_names: BTreeSet::new(),
        custom_providers: BTreeSet::new(),
        environment: BTreeMap::new(),
        registry: JSONProviderRegistry::default(),
        policies: ProviderPolicies::default(),
        globals: GlobalCredentials::default(),
        normalizers: name_normalizers(),
    };
    let lookup = CatalogLookup::new(&asset, Arc::new(catalog), settings)
        .map_err(|error| helper_error("CatalogError", &error.to_string()))?;
    Ok(CaseContext { lookup })
}

fn usage_input(source: &UsageInput) -> Option<RequestUsage> {
    let value = serde_json::to_value(source).ok()?;
    let raw: RawUsageObject = serde_json::from_value(value).ok()?;
    request_usage(Some(&raw))
}

// A supplied price table is used as given; the source's nonnull semantics keep an empty table supplied.
fn priced(record: Option<&PriceRecord>) -> Option<PricedModel> {
    record.cloned().map(|table| PricedModel {
        record: table,
        tiered_pricing: Vec::new(),
        off_peak_pricing: None,
        off_peak_cache_read_present: false,
    })
}

fn helper_error(class: &str, message: &str) -> Box<HelperError> {
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: message.to_owned(),
    })
}
