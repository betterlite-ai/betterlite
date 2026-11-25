use std::io::Error as IoError;

use async_trait::async_trait;
use betterlite_models::decisions::Json;
use betterlite_models::model_catalog_keys::ModelCostKeys;
use betterlite_models::model_registration::{JsonObject, ModelCostEntries, ModelRegistry};
use betterlite_models::provider_model_sets::ProviderModelSets;
use betterlite_server::model_info_search::datetime::normalize_model_info_datetime;
use betterlite_server::model_info_search::enrich::{
    EnrichContext, EnrichError, ModelInfoCatalog, RouterSource, enrich_model_info,
};
use betterlite_server::model_info_search::filters::{
    is_auto_router_model, matches_model_info_filters, model_in_access_group,
};
use betterlite_server::model_info_search::paginate::paginate_models;
use betterlite_server::model_info_search::sort::sort_models;
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::runtime::Builder;

use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

#[derive(Debug, thiserror::Error)]
enum Failure {
    #[error(transparent)]
    Enrich(#[from] EnrichError),
    #[error("the {0} function needs a value")]
    Missing(&'static str),
    #[error("the runtime did not start: {0}")]
    Runtime(IoError),
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Function {
    Enrich,
    Sort,
    IsAutoRouter,
    ModelInAccessGroup,
    MatchesFilters,
    Paginate,
    Datetime,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CatalogEntry {
    model: String,
    #[serde(default)]
    provider: Option<String>,
    info: JsonObject,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RouterRecord {
    #[serde(default)]
    async_client: Option<String>,
    #[serde(default)]
    discovered: JsonObject,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    function: Function,
    #[serde(default)]
    model: Option<JsonObject>,
    #[serde(default)]
    debug: Option<bool>,
    #[serde(default)]
    router: Option<RouterRecord>,
    #[serde(default)]
    catalog: Vec<CatalogEntry>,
    #[serde(default)]
    cost_map: JsonObject,
    #[serde(default)]
    models: Vec<JsonObject>,
    #[serde(default)]
    sort_by: Option<String>,
    #[serde(default)]
    sort_order: Option<String>,
    #[serde(default)]
    access_group: Option<String>,
    #[serde(default)]
    exclude_auto_routers: Option<bool>,
    #[serde(default)]
    wildcard_only: Option<bool>,
    #[serde(default)]
    page: Option<i64>,
    #[serde(default)]
    size: Option<i64>,
    #[serde(default)]
    total_count: Option<i64>,
    #[serde(default)]
    search: Option<String>,
    #[serde(default)]
    value: Option<Json>,
}

struct CaseCatalog<'entries> {
    entries: &'entries [CatalogEntry],
}

#[async_trait]
impl ModelInfoCatalog for CaseCatalog<'_> {
    async fn model_info(&self, model: &str, provider: Option<&str>) -> Option<JsonObject> {
        self.entries
            .iter()
            .find(|entry| entry.model == model && entry.provider.as_deref() == provider)
            .map(|entry| entry.info.clone())
    }
}

impl RouterSource for RouterRecord {
    fn async_client_name(&self, _model_id: Option<&str>) -> Option<String> {
        self.async_client.clone()
    }

    fn discovered_model_info(&self, _model_id: Option<&str>) -> JsonObject {
        self.discovered.clone()
    }
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        answer(input).or_value_error()
    }
}

fn answer(input: &Input) -> Result<Value, Failure> {
    match input.function {
        Function::Enrich => enrich_answer(input),
        Function::Sort => Ok(json!(sort_models(
            &input.models,
            input.sort_by.as_deref(),
            input.sort_order.as_deref(),
        ))),
        Function::IsAutoRouter => Ok(per_model(input, is_auto_router_model)),
        Function::ModelInAccessGroup => access_group_answer(input),
        Function::MatchesFilters => Ok(per_model(input, |model| {
            matches_model_info_filters(
                model,
                input.exclude_auto_routers,
                input.access_group.as_deref(),
                input.wildcard_only,
            )
        })),
        Function::Paginate => paginate_answer(input),
        Function::Datetime => Ok(json!(
            input
                .value
                .as_ref()
                .and_then(normalize_model_info_datetime)
                .map(|found| isoformat(&found))
        )),
    }
}

fn per_model<Test>(input: &Input, test: Test) -> Value
where
    Test: Fn(&JsonObject) -> bool,
{
    let mut verdicts = Vec::with_capacity(input.models.len());
    for model in &input.models {
        verdicts.push(test(model));
    }
    json!(verdicts)
}

fn access_group_answer(input: &Input) -> Result<Value, Failure> {
    let group = input
        .access_group
        .as_deref()
        .ok_or(Failure::Missing("model_in_access_group"))?;
    Ok(per_model(input, |model| {
        model_in_access_group(model, group)
    }))
}

fn enrich_answer(input: &Input) -> Result<Value, Failure> {
    let model = input.model.as_ref().ok_or(Failure::Missing("enrich"))?;
    let runtime = Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(Failure::Runtime)?;
    let registry = ModelRegistry {
        model_cost: input.cost_map.clone(),
        runtime_registered: ModelCostEntries::default(),
        provider_sets: ProviderModelSets::default(),
        keys: ModelCostKeys::default(),
        live_replay: None,
    };
    let catalog = CaseCatalog {
        entries: &input.catalog,
    };
    let context = EnrichContext {
        registry: &registry,
        catalog: &catalog,
        router: input
            .router
            .as_ref()
            .map(|record| -> &dyn RouterSource { record }),
    };
    let enriched = runtime.block_on(enrich_model_info(
        model,
        input.debug.unwrap_or(false),
        &context,
    ))?;
    Ok(json!(enriched))
}

fn paginate_answer(input: &Input) -> Result<Value, Failure> {
    let page = input.page.ok_or(Failure::Missing("paginate"))?;
    let size = input.size.ok_or(Failure::Missing("paginate"))?;
    let paginated = paginate_models(
        &input.models,
        page,
        size,
        input.total_count,
        input.search.as_deref(),
    );
    Ok(json!({
        "data": paginated.data,
        "total_count": paginated.total_count,
        "current_page": paginated.current_page,
        "total_pages": paginated.total_pages,
        "size": paginated.size,
    }))
}

fn isoformat(found: &DateTime<Utc>) -> String {
    if found.timestamp_subsec_micros() == 0 {
        found.format("%Y-%m-%dT%H:%M:%S+00:00").to_string()
    } else {
        found.format("%Y-%m-%dT%H:%M:%S%.6f+00:00").to_string()
    }
}
