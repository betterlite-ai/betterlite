use std::collections::BTreeMap;
use std::fmt::{Display, Write as _};
use std::future::{Future, ready};
use std::sync::{Arc, Mutex};

use betterlite_models::catalog_generalizations::CatalogGeneralizations;
use betterlite_models::decisions::Json;
use betterlite_models::model_catalog_keys::ModelCostKeys;
use betterlite_models::model_registration::{
    CostMapFetcher, JsonObject, ModelCostEntries, ModelInfoLookup, ModelRegistry,
    RegistrationEnvironment,
};

use crate::catalog_updates::catalog::{CatalogHandle, DeploymentCatalog};
use crate::catalog_updates::entries::{ModelEntry, deployment_inputs};
use crate::catalog_updates::updater::{CatalogEffects, CatalogUpdater};
use crate::deployment_identity::construct::{Construction, ConstructionPolicy, create_deployment};
use crate::deployment_identity::error::InvalidDeployment;
use crate::deployment_identity::spec::DeploymentSpec;
use crate::deployment_pricing::PricingInputs;
use crate::wildcard_catalog::PatternCatalog;

struct NoModelInfo;

impl ModelInfoLookup for NoModelInfo {
    fn get_model_info(
        &self,
        _model: &str,
        _model_cost: &JsonObject,
        _keys: &mut ModelCostKeys,
    ) -> impl Future<Output = Option<JsonObject>> + Send {
        ready(None)
    }

    fn clear_caches(&self) {}
}

struct NoFetch;

#[derive(Debug, thiserror::Error)]
#[error("these tests never fetch a cost map")]
struct NoFetchError;

impl CostMapFetcher for NoFetch {
    type Error = NoFetchError;

    fn get_model_cost_map(
        &self,
        _url: &str,
        _max_attempts: u32,
    ) -> impl Future<Output = Result<ModelCostEntries, NoFetchError>> + Send {
        ready(Err(NoFetchError))
    }
}

struct Allow;

impl ConstructionPolicy for Allow {
    fn check_access_windows(&self, _: &JsonObject, _: &str) -> Result<(), InvalidDeployment> {
        Ok(())
    }

    fn warn_credential_mismatch(&self, _: &str, _: &JsonObject) {}

    fn is_active_for_environment(&self, _: &JsonObject) -> Result<bool, InvalidDeployment> {
        Ok(true)
    }
}

// Records what the update asked of the other tasks. A name starting with "unresolved" or "bad" fails the admit, and "flaky" fails every admit after its first.
#[derive(Default)]
struct Recorder(Mutex<Vec<String>>);

impl Recorder {
    fn note(&self, line: String) {
        self.0.lock().unwrap().push(line);
    }

    fn count(&self, line: &str) -> usize {
        self.0
            .lock()
            .unwrap()
            .iter()
            .filter(|seen| *seen == line)
            .count()
    }
}

impl CatalogEffects for Recorder {
    fn admit(&self, spec: &DeploymentSpec) -> Result<(), InvalidDeployment> {
        let line = format!("admit:{}", spec.model_name);
        let earlier = self.count(&line);
        self.note(line);
        if spec.model_name == "flaky" && earlier > 0 {
            return Err(InvalidDeployment::Rejected {
                message: "flaky again".to_owned(),
            });
        }
        if spec.model_name.starts_with("unresolved") {
            return Err(InvalidDeployment::ProviderUnresolved {
                message: format!("no provider for {}", spec.model_name),
            });
        }
        if spec.model_name.starts_with("bad") {
            return Err(InvalidDeployment::Rejected {
                message: format!("rejected {}", spec.model_name),
            });
        }
        Ok(())
    }

    fn registered(&self, spec: &DeploymentSpec) {
        self.note(format!("registered:{}", spec.id().unwrap_or_default()));
    }

    fn released(&self, spec: &DeploymentSpec) {
        self.note(format!("released:{}", spec.id().unwrap_or_default()));
    }

    fn participates_in_adaptive(&self, spec: &DeploymentSpec) -> bool {
        spec.model_name.starts_with("adaptive")
    }

    fn finalize_adaptive(&self, catalog: &DeploymentCatalog) {
        self.note(format!("adaptive:{}", catalog.models.len()));
    }

    fn invalidated(&self) {
        self.note("invalidated".to_owned());
    }
}

pub(super) fn entry(name: &str, id: &str, model: &str) -> String {
    format!(
        r#"{{"model_name":"{name}","litellm_params":{{"model":"{model}"}},"model_info":{{"id":"{id}"}}}}"#
    )
}

pub(super) fn team_entry(name: &str, id: &str, team: &str, public: &str) -> String {
    format!(
        r#"{{"model_name":"{name}","litellm_params":{{"model":"openai/m"}},"model_info":{{"id":"{id}","team_id":"{team}","team_public_model_name":"{public}"}}}}"#
    )
}

pub(super) enum Step {
    Set(Vec<String>),
    Add(String),
    Upsert(String),
    Delete(&'static str),
    Hold,
    Inputs(Vec<String>),
}

#[derive(Clone, Copy)]
pub(super) enum Show {
    Outcomes,
    Models,
    Patterns,
    Teams,
    Calls,
    Held,
}

fn generated_id() -> String {
    "generated-id".to_owned()
}

fn never(_: &str) -> bool {
    false
}

const PRICING: PricingInputs = PricingInputs {
    ptu_cost_attribution: false,
    is_strategy_router: never,
};

// model_name:model_info id:declared id:duplicate ids:organization, one input per segment.
fn rendered_inputs(entries: &[ModelEntry]) -> String {
    deployment_inputs(entries)
        .iter()
        .map(|input| {
            let organization = match input.litellm_params.get("organization") {
                Some(Json::Text(text)) => text.clone(),
                _ => "-".to_owned(),
            };
            format!(
                "{}:{}:{}:{}:{organization}",
                input.model_name,
                serde_json::to_string(&input.model_info.get("id")).unwrap(),
                input.declared_id.as_deref().unwrap_or("-"),
                input
                    .duplicate_ids
                    .iter()
                    .cloned()
                    .collect::<Vec<_>>()
                    .join("+"),
            )
        })
        .collect::<Vec<_>>()
        .join("|")
}

fn parsed(texts: &[String]) -> Vec<ModelEntry> {
    texts
        .iter()
        .map(|text| serde_json::from_str(text).unwrap())
        .collect()
}

// A deployment as set_model_list builds it, so an upsert compares like with like.
async fn built(
    run: &mut Construction<'_, NoModelInfo, NoFetch, Allow>,
    text: &str,
) -> DeploymentSpec {
    let inputs = deployment_inputs(&parsed(&[text.to_owned()]));
    let created = create_deployment(run, inputs.first().unwrap())
        .await
        .unwrap();
    created.deployment.unwrap()
}

fn id_of(spec: Option<DeploymentSpec>) -> String {
    spec.map_or_else(
        || "None".to_owned(),
        |found| format!("Some({})", found.id().unwrap_or_default()),
    )
}

fn indices<K>(map: &BTreeMap<K, Vec<usize>>) -> String
where
    K: Display,
{
    map.iter()
        .map(|(key, found)| format!("{key}:{found:?}"))
        .collect::<Vec<_>>()
        .join(",")
}

fn route(patterns: &PatternCatalog) -> String {
    let ids: Vec<String> = patterns
        .route("x/y", None)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|candidate| candidate.model_id)
        .collect();
    ids.join("+")
}

fn joined<I>(items: I) -> String
where
    I: IntoIterator,
    I::Item: Display,
{
    let texts: Vec<String> = items.into_iter().map(|item| item.to_string()).collect();
    texts.join(",")
}

fn render_models(catalog: &DeploymentCatalog) -> String {
    let mut listed = String::new();
    for spec in &catalog.models {
        write!(
            listed,
            "{}:{},",
            spec.model_name,
            spec.id().unwrap_or_default()
        )
        .unwrap();
    }
    let mut keyed = String::new();
    for (id, index) in &catalog.id_index {
        write!(keyed, "{id}:{index},").unwrap();
    }
    let names = indices(&catalog.name_index);
    let set = joined(&catalog.model_names);
    let (models, ids) = (listed.trim_end_matches(','), keyed.trim_end_matches(','));
    format!("models=[{models}] ids({ids}) names({names}) set=[{set}]")
}

fn render(catalog: &DeploymentCatalog, show: Show) -> String {
    let teams = catalog
        .team_pattern_routers
        .iter()
        .map(|(team, patterns)| format!("{team}:{}", route(&patterns.router)));
    let unresolved = catalog
        .provider_unresolved
        .iter()
        .map(|input| &input.model_name);
    let keys = catalog
        .team_index
        .iter()
        .map(|((team, name), found)| format!("{team}/{name}:{found:?}"));
    match show {
        Show::Models | Show::Held => render_models(catalog),
        Show::Patterns => format!(
            "defaults=[{}] route=[{}] teams=[{}] unresolved=[{}]",
            joined(&catalog.provider_default_ids),
            route(&catalog.pattern_router),
            joined(teams),
            joined(unresolved)
        ),
        Show::Teams => format!(
            "teams=[{}] public=[{}]",
            joined(keys),
            joined(catalog.team_public_model_names())
        ),
        Show::Outcomes | Show::Calls => String::new(),
    }
}

type TestUpdater<'run> = CatalogUpdater<'run, NoModelInfo, NoFetch, Allow, Recorder>;

// Runs one step and says what it answered.
async fn apply(updater: &mut TestUpdater<'_>, step: Step) -> String {
    match step {
        Step::Set(texts) => updater.set_model_list(&parsed(&texts)).await.map_or_else(
            |failure| format!("set:err({failure})"),
            |()| "set:ok".to_owned(),
        ),
        Step::Add(text) => {
            let spec = built(&mut updater.construction, &text).await;
            updater.add_deployment(spec).await.map_or_else(
                |failure| format!("add:err({failure})"),
                |added| format!("add:{}", id_of(added)),
            )
        }
        Step::Upsert(text) => {
            let spec = built(&mut updater.construction, &text).await;
            updater.upsert_deployment(spec).await.map_or_else(
                |failure| format!("upsert:err({failure})"),
                |upserted| format!("upsert:{}", id_of(upserted)),
            )
        }
        Step::Delete(id) => format!("delete:{}", id_of(updater.delete_deployment(id))),
        Step::Inputs(texts) => format!("inputs:{}", rendered_inputs(&parsed(&texts))),
        Step::Hold => "hold".to_owned(),
    }
}

// Runs the steps against one handle and renders what the caller asked to see.
pub(super) async fn observe(steps: Vec<Step>, ignore: bool, show: Show) -> String {
    let handle = CatalogHandle::default();
    let effects = Recorder::default();
    let mut registry = ModelRegistry::default();
    let generalizations = CatalogGeneralizations::default();
    let environment = RegistrationEnvironment {
        lookup: &NoModelInfo,
        fetcher: &NoFetch,
        generalizations: &generalizations,
        bedrock_base_model: str::to_owned,
    };
    let mut outcomes = Vec::new();
    let mut held: Vec<Arc<DeploymentCatalog>> = Vec::new();
    for step in steps {
        if matches!(step, Step::Hold) {
            held.push(handle.load_full());
        }
        let mut updater = CatalogUpdater {
            catalog: &handle,
            effects: &effects,
            construction: Construction {
                registry: &mut registry,
                environment: &environment,
                pricing: &PRICING,
                policy: &Allow,
                new_id: generated_id,
                ignore_invalid_deployments: ignore,
            },
        };
        outcomes.push(apply(&mut updater, step).await);
    }
    match show {
        Show::Outcomes => outcomes.join(";"),
        Show::Calls => effects.0.lock().unwrap().join(","),
        Show::Held => held
            .first()
            .map_or_else(String::new, |old| render(old, show)),
        Show::Models | Show::Patterns | Show::Teams => render(&handle.load_full(), show),
    }
}
