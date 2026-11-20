#![cfg(test)]

use std::collections::BTreeMap;
use std::convert::Infallible;
use std::future::{Future, ready};
use std::io;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_context::anthropic_catalog_lookup::RawCatalog;
use betterlite_models::decisions::Json;
use betterlite_models::full_model_metadata::parameters::SupportedParameterSource;
use betterlite_models::metadata::levels::ReasoningEffort;
use betterlite_models::model_catalog_keys::ModelCostKeys;
use betterlite_models::model_metadata_resolution::names::ModelCostMap;
use betterlite_models::model_registration::JsonObject;
use rust_decimal::Decimal;
use serde_json::Number;
use tokio::runtime::Builder;

use crate::catalog_updates::catalog::DeploymentCatalog;
use crate::deployment_identity::spec::DeploymentSpec;
use crate::model_catalog_lookup::alias::{AliasMap, AliasTarget};
use crate::model_catalog_lookup::error::LookupError;
use crate::model_catalog_lookup::lookup::CatalogLookup;
use crate::model_group_info::aggregate::get_model_group_info;
use crate::model_group_info::error::GroupInfoError;
use crate::model_group_info::group::{Capability, ClientsideAuth, ModelGroupInfo};
use crate::model_group_info::sources::{
    DeploymentInfo, GroupSources, ProviderAnswer, ProviderFailure, ProviderRequest,
    ProviderResolver,
};
use crate::routing_groups::index::RoutingGroupIndex;
use tracing::subscriber::{DefaultGuard, set_default};

#[derive(Clone, Default)]
struct Logs(Arc<Mutex<Vec<u8>>>);

impl io::Write for Logs {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let mut logs = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        logs.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Logs {
    fn text(&self) -> String {
        String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
    }
}

fn capture_logs() -> (Logs, DefaultGuard) {
    let logs = Logs::default();
    let writer = logs.clone();
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_writer(move || writer.clone())
        .finish();
    (logs, set_default(subscriber))
}

#[derive(Debug, thiserror::Error)]
#[error("the deployment metadata lookup failed")]
struct Unavailable;

#[derive(Default)]
struct FakeInfo {
    answers: BTreeMap<String, JsonObject>,
    failing: bool,
}

impl DeploymentInfo for FakeInfo {
    type Error = Unavailable;

    fn info(
        &self,
        model_id: &str,
        _model_name: &str,
        _keys: &mut ModelCostKeys,
    ) -> impl Future<Output = Result<Option<JsonObject>, Unavailable>> + Send {
        if self.failing {
            return ready(Err(Unavailable));
        }
        ready(Ok(self.answers.get(model_id).cloned()))
    }
}

#[derive(Default)]
struct FakeProvider {
    answers: BTreeMap<String, (String, String)>,
    failure: Option<ProviderFailure>,
}

impl ProviderResolver for FakeProvider {
    fn resolve_provider(
        &self,
        request: &ProviderRequest<'_>,
    ) -> Result<ProviderAnswer, ProviderFailure> {
        if let Some(failure) = &self.failure {
            return Err(failure.clone());
        }
        match self.answers.get(request.model) {
            Some((litellm_model, provider)) => Ok(ProviderAnswer {
                litellm_model: litellm_model.clone(),
                provider: provider.clone(),
            }),
            None => Err(ProviderFailure::BadRequest(format!(
                "LLM Provider NOT provided. Pass in the LLM provider you are trying to call. You passed model={}",
                request.model
            ))),
        }
    }
}

#[derive(Default)]
struct FakeParams {
    answers: BTreeMap<String, Option<Vec<String>>>,
}

impl SupportedParameterSource for FakeParams {
    type Error = Infallible;

    fn supported_openai_params(
        &self,
        model: &str,
        custom_llm_provider: Option<&str>,
    ) -> Result<Option<Vec<String>>, Infallible> {
        let key = custom_llm_provider.map_or_else(
            || model.to_owned(),
            |provider| format!("{provider}/{model}"),
        );
        let found = self.answers.get(&key).or_else(|| self.answers.get(model));
        Ok(found.cloned().flatten())
    }
}

struct World {
    catalog: DeploymentCatalog,
    aliases: AliasMap,
    groups: RoutingGroupIndex,
}

struct Scenario {
    world: World,
    info: FakeInfo,
    providers: FakeProvider,
    parameters: FakeParams,
    cost_map: ModelCostMap,
    fast_mode: RawCatalog,
}

impl Scenario {
    fn new(rows: &[&str]) -> Self {
        Self::from_specs(rows.iter().map(|text| spec(text)).collect())
    }

    fn from_specs(specs: Vec<DeploymentSpec>) -> Self {
        Self {
            world: world(specs),
            info: FakeInfo::default(),
            providers: FakeProvider::default(),
            parameters: FakeParams::default(),
            cost_map: cost_map("{}"),
            fast_mode: RawCatalog::default(),
        }
    }

    fn group(&self, model_group: &str) -> Result<Option<ModelGroupInfo>, GroupInfoError> {
        let lookup = CatalogLookup {
            catalog: &self.world.catalog,
            model_group_alias: &self.world.aliases,
            routing_groups: &self.world.groups,
            inferred_provider: &no_inferred,
        };
        let sources = GroupSources {
            deployment_info: &self.info,
            providers: &self.providers,
            parameters: &self.parameters,
            cost_map: &self.cost_map,
            fast_mode_catalog: &self.fast_mode,
        };
        let mut keys = ModelCostKeys::default();
        block_on(get_model_group_info(
            &lookup,
            &sources,
            model_group,
            &mut keys,
        ))
    }

    fn answer(&mut self, model: &str, litellm_model: &str, provider: &str) {
        self.providers.answers.insert(
            model.to_owned(),
            (litellm_model.to_owned(), provider.to_owned()),
        );
    }

    fn metadata(&mut self, id: &str, info: &str) {
        self.info.answers.insert(id.to_owned(), parse(info));
    }
}

fn block_on<F>(future: F) -> F::Output
where
    F: Future,
{
    Builder::new_current_thread()
        .build()
        .expect("runtime")
        .block_on(future)
}

const fn no_inferred(_model: &str) -> Option<String> {
    None
}

fn spec(text: &str) -> DeploymentSpec {
    let parsed = parse(text);
    DeploymentSpec::new(
        text_field(&parsed, "model_name"),
        object_field(&parsed, "litellm_params"),
        object_field(&parsed, "model_info"),
        object_field(&parsed, "extra"),
        generated_id,
    )
    .expect("deployment")
}

fn generated_id() -> String {
    "generated".to_owned()
}

fn world(specs: Vec<DeploymentSpec>) -> World {
    let mut catalog = DeploymentCatalog::default();
    for spec in specs {
        catalog.index_patterns(&spec).expect("pattern index");
        catalog.push(spec);
    }
    World {
        catalog,
        aliases: AliasMap::new(),
        groups: RoutingGroupIndex::default(),
    }
}

fn without_id(mut spec: DeploymentSpec) -> DeploymentSpec {
    spec.model_info.shift_remove("id");
    spec
}

fn parse(text: &str) -> JsonObject {
    serde_json::from_str(text).expect("json object")
}

fn text_field(parsed: &JsonObject, key: &str) -> String {
    match parsed.get(key) {
        Some(Json::Text(text)) => text.clone(),
        _ => String::new(),
    }
}

fn object_field(parsed: &JsonObject, key: &str) -> JsonObject {
    match parsed.get(key) {
        Some(Json::Object(fields)) => fields.clone(),
        _ => JsonObject::new(),
    }
}

fn cost_map(text: &str) -> ModelCostMap {
    ModelCostMap::parse(text.as_bytes()).expect("cost map")
}

fn fast_mode_catalog(text: &str) -> RawCatalog {
    serde_json::from_str(text).expect("fast mode catalog")
}

fn decimal(text: &str) -> Decimal {
    Decimal::from_str_exact(text).expect("decimal")
}

fn group_of(scenario: &Scenario, name: &str) -> ModelGroupInfo {
    scenario.group(name).expect("group info").expect("group")
}

fn alias_item(scenario: &mut Scenario, alias: &str, model: Option<Json>, hidden: Option<Json>) {
    scenario
        .world
        .aliases
        .insert(alias.to_owned(), AliasTarget::Item { model, hidden });
}

const ALPHA: &str =
    r#"{"model_name":"g","litellm_params":{"model":"openai/b846-alpha"},"model_info":{"id":"a"}}"#;
const BETA: &str = r#"{"model_name":"g","litellm_params":{"model":"anthropic/b846-beta"},"model_info":{"id":"b"}}"#;

#[test]
fn mixed_provider_group() {
    let mut scenario = Scenario::new(&[ALPHA, BETA]);
    scenario.metadata(
        "a",
        r#"{"mode":"chat","max_input_tokens":1000,"max_output_tokens":100,"input_cost_per_token":0.1,"output_cost_per_token":0.2,"supports_vision":true}"#,
    );
    scenario.metadata(
        "b",
        r#"{"mode":"chat","max_input_tokens":2000,"max_output_tokens":50,"input_cost_per_token":0.05,"output_cost_per_token":0.4,"supports_function_calling":true}"#,
    );
    scenario.answer("openai/b846-alpha", "b846-alpha", "openai");
    scenario.answer("anthropic/b846-beta", "b846-beta", "anthropic");

    let group = group_of(&scenario, "g");
    assert_eq!(group.model_group, "g");
    assert_eq!(
        group.providers,
        vec!["openai".to_owned(), "anthropic".to_owned()]
    );
    assert_eq!(group.max_input_tokens, Some(decimal("2000")));
    assert_eq!(group.max_output_tokens, Some(decimal("100")));
    assert_eq!(group.input_cost_per_token, Some(decimal("0.1")));
    assert_eq!(group.output_cost_per_token, Some(decimal("0.4")));
    assert_eq!(group.mode.as_deref(), Some("chat"));
    assert_eq!(
        group.capabilities,
        [Capability::Vision, Capability::FunctionCalling]
            .into_iter()
            .collect()
    );
    assert!(!group.supports_fast_mode);
    assert_eq!(group.supported_reasoning_efforts, Some(Vec::new()));
    assert_eq!(group.supported_openai_params, Some(Vec::new()));
    assert_eq!(group.clientside_auth, None);
}

#[test]
fn widest_token_limit() {
    let mut scenario = Scenario::new(&[
        r#"{"model_name":"g","litellm_params":{"model":"openai/m1"},"model_info":{"id":"a"}}"#,
        r#"{"model_name":"g","litellm_params":{"model":"openai/m2"},"model_info":{"id":"b"}}"#,
        r#"{"model_name":"g","litellm_params":{"model":"openai/m3"},"model_info":{"id":"c"}}"#,
    ]);
    scenario.metadata("a", r#"{"mode":"chat"}"#);
    scenario.metadata(
        "b",
        r#"{"mode":"chat","max_input_tokens":500,"max_output_tokens":20}"#,
    );
    scenario.metadata("c", r#"{"mode":"chat","max_input_tokens":900}"#);
    scenario.answer("openai/m1", "m1", "openai");
    scenario.answer("openai/m2", "m2", "openai");
    scenario.answer("openai/m3", "m3", "openai");

    let group = group_of(&scenario, "g");
    assert_eq!(group.max_input_tokens, Some(decimal("900")));
    assert_eq!(group.max_output_tokens, Some(decimal("20")));
}

#[test]
fn capability_merge() {
    let mut scenario = Scenario::new(&[ALPHA, BETA]);
    scenario.metadata(
        "a",
        r#"{"mode":"chat","supports_vision":true,"supports_parallel_function_calling":true,"supported_openai_params":["temperature"]}"#,
    );
    scenario.metadata(
        "b",
        r#"{"mode":"chat","supports_vision":false,"supports_function_calling":true,"supports_web_search":true,"supported_openai_params":[]}"#,
    );
    scenario.answer("openai/b846-alpha", "b846-alpha", "openai");
    scenario.answer("anthropic/b846-beta", "b846-beta", "anthropic");

    let group = group_of(&scenario, "g");
    assert!(group.capabilities.contains(&Capability::Vision));
    assert!(
        group
            .capabilities
            .contains(&Capability::ParallelFunctionCalling)
    );
    assert!(group.capabilities.contains(&Capability::FunctionCalling));
    assert!(group.capabilities.contains(&Capability::WebSearch));
    assert!(!group.capabilities.contains(&Capability::UrlContext));
    assert_eq!(group.supported_openai_params, Some(Vec::new()));

    scenario.metadata("b", r#"{"mode":"chat","supported_openai_params":null}"#);
    let kept = group_of(&scenario, "g");
    assert_eq!(
        kept.supported_openai_params,
        Some(vec!["temperature".to_owned()])
    );
}

#[test]
fn group_cost() {
    let mut scenario = Scenario::new(&[ALPHA, BETA]);
    scenario.metadata(
        "a",
        r#"{"mode":"chat","input_cost_per_token":0.5,"output_cost_per_token":0.25,"input_cost_per_pixel":0.03}"#,
    );
    scenario.metadata(
        "b",
        r#"{"mode":"chat","input_cost_per_token":0.1,"output_cost_per_token":0.9,"input_cost_per_pixel":0.5}"#,
    );
    scenario.answer("openai/b846-alpha", "b846-alpha", "openai");
    scenario.answer("anthropic/b846-beta", "b846-beta", "anthropic");

    let group = group_of(&scenario, "g");
    assert_eq!(group.input_cost_per_token, Some(decimal("0.5")));
    assert_eq!(group.output_cost_per_token, Some(decimal("0.9")));
    assert_eq!(group.input_cost_per_pixel, Some(decimal("0.03")));
}

#[test]
fn unknown_group() {
    let scenario = Scenario::new(&[]);
    assert_eq!(scenario.group("missing").expect("info"), None);
}

#[test]
fn deployment_order_effects() {
    let mut scenario = Scenario::new(&[ALPHA, BETA]);
    scenario.metadata(
        "a",
        r#"{"mode":"embedding","tpm":77,"rpm":88,"itpm":5,"otpm":6}"#,
    );
    scenario.metadata("b", r#"{"mode":"chat","tpm":99,"rpm":1}"#);
    scenario.answer("openai/b846-alpha", "b846-alpha", "openai");
    scenario.answer("anthropic/b846-beta", "b846-beta", "anthropic");
    scenario
        .world
        .catalog
        .models
        .first_mut()
        .expect("row")
        .litellm_params
        .insert(
            "configurable_clientside_auth_params".to_owned(),
            Json::List(vec![Json::Text("temperature".to_owned())]),
        );

    let group = group_of(&scenario, "g");
    assert_eq!(group.mode.as_deref(), Some("embedding"));
    assert_eq!(group.tpm, Some(99));
    assert_eq!(group.rpm, Some(1));
    assert_eq!(group.itpm, Some(5));
    assert_eq!(group.otpm, Some(6));
    assert_eq!(group.clientside_auth, None);
}

#[test]
fn first_row_limit_asymmetry() {
    let mut scenario = Scenario::new(&[ALPHA]);
    scenario.metadata("a", r#"{"mode":"chat","tpm":77,"rpm":88}"#);
    scenario.answer("openai/b846-alpha", "b846-alpha", "openai");

    let group = group_of(&scenario, "g");
    assert_eq!(group.tpm, Some(77));
    assert_eq!(group.rpm, Some(88));

    let mut two = Scenario::new(&[ALPHA, BETA]);
    two.metadata("a", r#"{"mode":"chat","tpm":77}"#);
    two.metadata("b", r#"{"mode":"chat","tpm":999}"#);
    two.answer("openai/b846-alpha", "b846-alpha", "openai");
    two.answer("anthropic/b846-beta", "b846-beta", "anthropic");
    two.world
        .catalog
        .models
        .get_mut(1)
        .expect("row")
        .extra
        .insert("tpm".to_owned(), Json::Number(Number::from(20_i64)));
    assert_eq!(group_of(&two, "g").tpm, Some(20));
}

#[test]
fn limit_source_precedence() {
    let mut scenario = Scenario::new(&[ALPHA]);
    scenario.metadata("a", r#"{"mode":"chat"}"#);
    scenario.answer("openai/b846-alpha", "b846-alpha", "openai");
    let row = scenario.world.catalog.models.first_mut().expect("row");
    row.extra
        .insert("tpm".to_owned(), Json::Number(Number::from(30_i64)));
    row.litellm_params
        .insert("tpm".to_owned(), Json::Number(Number::from(20_i64)));
    row.litellm_params
        .insert("itpm".to_owned(), Json::Number(Number::from(7_i64)));
    row.model_info
        .insert("tpm".to_owned(), Json::Number(Number::from(10_i64)));
    row.model_info
        .insert("rpm".to_owned(), Json::Number(Number::from(4_i64)));
    row.model_info
        .insert("otpm".to_owned(), Json::Number(Number::from(8_i64)));

    let group = group_of(&scenario, "g");
    assert_eq!(group.tpm, Some(30));
    assert_eq!(group.rpm, Some(4));
    assert_eq!(group.itpm, Some(7));
    assert_eq!(group.otpm, Some(8));
}

#[test]
fn zero_and_null_totals() {
    let mut scenario = Scenario::new(&[ALPHA, BETA]);
    scenario.metadata("a", r#"{"mode":"chat"}"#);
    scenario.metadata("b", r#"{"mode":"chat"}"#);
    scenario.answer("openai/b846-alpha", "b846-alpha", "openai");
    scenario.answer("anthropic/b846-beta", "b846-beta", "anthropic");
    scenario
        .world
        .catalog
        .models
        .first_mut()
        .expect("row")
        .model_info
        .insert("tpm".to_owned(), Json::Number(Number::from(0_i64)));
    scenario
        .world
        .catalog
        .models
        .get_mut(1)
        .expect("row")
        .model_info
        .insert("itpm".to_owned(), Json::Null);

    let group = group_of(&scenario, "g");
    assert_eq!(group.tpm, Some(0));
    assert_eq!(group.itpm, None);
}

#[test]
fn last_supported_params() {
    let mut scenario = Scenario::new(&[ALPHA, BETA]);
    scenario.metadata(
        "a",
        r#"{"mode":"chat","supported_openai_params":["temperature"]}"#,
    );
    scenario.metadata(
        "b",
        r#"{"mode":"chat","supported_openai_params":["top_p","max_tokens"]}"#,
    );
    scenario.answer("openai/b846-alpha", "b846-alpha", "openai");
    scenario.answer("anthropic/b846-beta", "b846-beta", "anthropic");

    let group = group_of(&scenario, "g");
    assert_eq!(
        group.supported_openai_params,
        Some(vec!["top_p".to_owned(), "max_tokens".to_owned()])
    );
}

#[test]
fn last_auth_param_null() {
    let mut scenario = Scenario::new(&[ALPHA, BETA]);
    scenario.metadata("a", r#"{"mode":"chat"}"#);
    scenario.metadata("b", r#"{"mode":"chat"}"#);
    scenario.answer("openai/b846-alpha", "b846-alpha", "openai");
    scenario.answer("anthropic/b846-beta", "b846-beta", "anthropic");
    scenario
        .world
        .catalog
        .models
        .first_mut()
        .expect("row")
        .litellm_params
        .insert(
            "configurable_clientside_auth_params".to_owned(),
            Json::List(vec![Json::Text("temperature".to_owned())]),
        );

    let group = group_of(&scenario, "g");
    assert_eq!(group.clientside_auth, None);
}

#[test]
fn last_auth_param_present() {
    let mut scenario = Scenario::new(&[ALPHA, BETA]);
    scenario.metadata("a", r#"{"mode":"chat"}"#);
    scenario.metadata("b", r#"{"mode":"chat"}"#);
    scenario.answer("openai/b846-alpha", "b846-alpha", "openai");
    scenario.answer("anthropic/b846-beta", "b846-beta", "anthropic");
    let row = scenario.world.catalog.models.get_mut(1).expect("row");
    row.litellm_params.insert(
        "configurable_clientside_auth_params".to_owned(),
        Json::List(vec![
            Json::Text("temperature".to_owned()),
            Json::Object(parse(r#"{"api_base":"https://b846","extra":1}"#)),
        ]),
    );

    let group = group_of(&scenario, "g");
    let auth = group.clientside_auth.expect("auth");
    assert_eq!(
        auth.first(),
        Some(&ClientsideAuth::Name("temperature".to_owned()))
    );
    assert_eq!(
        auth.get(1),
        Some(&ClientsideAuth::Item(parse(
            r#"{"api_base":"https://b846","extra":1}"#
        )))
    );
}

#[test]
fn constructor_auth_drops_extras() {
    let mut scenario = Scenario::new(&[ALPHA]);
    scenario.metadata(
        "a",
        r#"{"mode":"chat","configurable_clientside_auth_params":[{"api_base":"https://constructor","extra":2}]}"#,
    );
    scenario.answer("openai/b846-alpha", "b846-alpha", "openai");
    let group = group_of(&scenario, "g");
    assert_eq!(
        group.clientside_auth,
        Some(vec![ClientsideAuth::Item(parse(
            r#"{"api_base":"https://constructor"}"#
        ))])
    );
}

#[test]
fn first_mode_pixel_cost() {
    let mut scenario = Scenario::new(&[ALPHA, BETA]);
    scenario.metadata("a", r#"{"mode":"embedding","input_cost_per_pixel":0.03}"#);
    scenario.metadata("b", r#"{"mode":"chat","input_cost_per_pixel":0.9}"#);
    scenario.answer("openai/b846-alpha", "b846-alpha", "openai");
    scenario.answer("anthropic/b846-beta", "b846-beta", "anthropic");

    let group = group_of(&scenario, "g");
    assert_eq!(group.mode.as_deref(), Some("embedding"));
    assert_eq!(group.input_cost_per_pixel, Some(decimal("0.03")));
}

#[test]
fn type_defaults_and_null_bools() {
    let mut scenario = Scenario::new(&[ALPHA]);
    scenario.metadata(
        "a",
        r#"{"mode":null,"supports_vision":null,"supported_openai_params":null}"#,
    );
    scenario.answer("openai/b846-alpha", "b846-alpha", "openai");

    let group = group_of(&scenario, "g");
    assert_eq!(group.mode, None);
    assert!(!group.capabilities.contains(&Capability::Vision));
    assert_eq!(group.supported_openai_params, None);

    scenario.metadata("a", "{}");
    let defaults = group_of(&scenario, "g");
    assert_eq!(defaults.mode.as_deref(), Some("chat"));
    assert_eq!(defaults.supported_openai_params, Some(Vec::new()));
}

#[test]
fn metadata_identity_override() {
    let mut scenario = Scenario::new(&[ALPHA, BETA]);
    scenario.metadata(
        "a",
        r#"{"mode":"chat","model_group":"renamed","providers":["azure"]}"#,
    );
    scenario.metadata("b", r#"{"mode":"chat"}"#);
    scenario.answer("openai/b846-alpha", "b846-alpha", "openai");
    scenario.answer("anthropic/b846-beta", "b846-beta", "anthropic");

    let group = group_of(&scenario, "g");
    assert_eq!(group.model_group, "renamed");
    assert_eq!(
        group.providers,
        vec!["azure".to_owned(), "anthropic".to_owned()]
    );
}

#[test]
fn lookup_error_fallback() {
    let mut scenario = Scenario::new(&[ALPHA]);
    scenario.info.failing = true;
    scenario.answer("openai/b846-alpha", "b846-alpha", "openai");
    scenario.parameters.answers.insert(
        "openai/b846-alpha".to_owned(),
        Some(vec!["temperature".to_owned()]),
    );
    scenario
        .world
        .catalog
        .models
        .first_mut()
        .expect("row")
        .model_info
        .insert("mode".to_owned(), Json::Text("embedding".to_owned()));
    let row = scenario.world.catalog.models.first_mut().expect("row");
    row.model_info.insert(
        "input_cost_per_token".to_owned(),
        Json::Text("0.5".to_owned()),
    );
    let output_cost = Number::from_f64(0.25).expect("number");
    row.model_info.insert(
        "output_cost_per_token".to_owned(),
        Json::Number(output_cost),
    );

    let group = group_of(&scenario, "g");
    assert_eq!(group.mode.as_deref(), Some("embedding"));
    assert_eq!(group.input_cost_per_token, Some(decimal("0.5")));
    assert_eq!(group.output_cost_per_token, Some(decimal("0.25")));
    assert_eq!(
        group.supported_openai_params,
        Some(vec!["temperature".to_owned()])
    );
    assert_eq!(group.max_input_tokens, None);
    assert_eq!(group.supported_reasoning_efforts, None);
}

#[test]
fn missing_id_fallback() {
    let mut scenario = Scenario::from_specs(vec![without_id(spec(ALPHA))]);
    scenario.answer("openai/b846-alpha", "b846-alpha", "openai");
    let group = group_of(&scenario, "g");
    assert_eq!(group.mode.as_deref(), Some("chat"));
    assert_eq!(group.supported_openai_params, Some(Vec::new()));
    assert_eq!(group.max_input_tokens, None);
}

#[test]
fn provider_error_logged() {
    let scenario = Scenario::new(&[
        r#"{"model_name":"g","litellm_params":{"model":"b846-no-provider"},"model_info":{}}"#,
    ]);
    let (logs, _guard) = capture_logs();
    let group = group_of(&scenario, "g");
    assert_eq!(group.providers, vec![String::new()]);
    assert_eq!(group.mode.as_deref(), Some("chat"));
    assert_eq!(group.supported_openai_params, Some(Vec::new()));
    assert!(
        logs.text()
            .contains("litellm.router.py::get_model_group_info() - LLM Provider NOT provided")
    );
}

#[test]
fn provider_other_error() {
    let mut scenario = Scenario::new(&[ALPHA]);
    scenario.providers.failure = Some(ProviderFailure::Other("b846 other".to_owned()));
    let outcome = scenario.group("g");
    assert!(
        matches!(outcome, Err(GroupInfoError::Provider { message }) if message == "b846 other")
    );
}

#[test]
fn fallback_chat_and_cost() {
    let mut scenario = Scenario::new(&[ALPHA]);
    scenario.answer("openai/b846-alpha", "b846-alpha", "openai");
    scenario
        .world
        .catalog
        .models
        .first_mut()
        .expect("row")
        .model_info
        .insert(
            "input_cost_per_token".to_owned(),
            Json::Text("zero".to_owned()),
        );
    let group = group_of(&scenario, "g");
    assert_eq!(group.mode.as_deref(), Some("chat"));
    assert_eq!(group.input_cost_per_token, None);

    scenario
        .world
        .catalog
        .models
        .first_mut()
        .expect("row")
        .model_info
        .insert("mode".to_owned(), Json::Null);
    let nulled = group_of(&scenario, "g");
    assert_eq!(nulled.mode.as_deref(), Some("chat"));
}

mod wire;

#[test]
fn string_alias_name() {
    let mut scenario = Scenario::new(&[
        r#"{"model_name":"target","litellm_params":{"model":"openai/b846-target"},"model_info":{"id":"a"}}"#,
    ]);
    scenario.metadata("a", r#"{"mode":"chat"}"#);
    scenario.answer("openai/b846-target", "b846-target", "openai");
    scenario
        .world
        .aliases
        .insert("alias".to_owned(), AliasTarget::Name("target".to_owned()));
    let group = group_of(&scenario, "alias");
    assert_eq!(group.model_group, "alias");
    assert_eq!(group.providers, vec!["openai".to_owned()]);

    scenario
        .world
        .aliases
        .insert("empty".to_owned(), AliasTarget::Name(String::new()));
    assert_eq!(scenario.group("empty").expect("info"), None);
}

#[test]
fn hidden_alias_none() {
    let mut scenario = Scenario::new(&[
        r#"{"model_name":"target","litellm_params":{"model":"openai/b846-target"},"model_info":{"id":"a"}}"#,
    ]);
    scenario.metadata("a", r#"{"mode":"chat"}"#);
    scenario.answer("openai/b846-target", "b846-target", "openai");
    alias_item(
        &mut scenario,
        "hidden",
        Some(Json::Text("target".to_owned())),
        Some(Json::Bool(true)),
    );
    assert_eq!(scenario.group("hidden").expect("info"), None);

    alias_item(&mut scenario, "null-model", None, Some(Json::Bool(false)));
    assert_eq!(scenario.group("null-model").expect("info"), None);
}

#[test]
fn invalid_alias_kind() {
    let mut scenario = Scenario::new(&[]);
    scenario.world.aliases.insert(
        "number".to_owned(),
        AliasTarget::Unsupported(Json::Number(Number::from(5_i64))),
    );
    assert_eq!(scenario.group("number").expect("info"), None);
}

#[test]
fn malformed_alias_error() {
    let mut scenario = Scenario::new(&[]);
    alias_item(
        &mut scenario,
        "no-hidden",
        Some(Json::Text("target".to_owned())),
        None,
    );
    assert!(matches!(
        scenario.group("no-hidden"),
        Err(GroupInfoError::Lookup(LookupError::MalformedAlias {
            field: "hidden",
            ..
        }))
    ));
    alias_item(&mut scenario, "no-model", None, Some(Json::Bool(false)));
    assert!(matches!(
        scenario.group("no-model"),
        Err(GroupInfoError::Lookup(LookupError::MalformedAlias {
            field: "model",
            ..
        }))
    ));
    alias_item(
        &mut scenario,
        "number-model",
        Some(Json::Number(Number::from(5_i64))),
        Some(Json::Bool(false)),
    );
    assert!(matches!(
        scenario.group("number-model"),
        Err(GroupInfoError::AliasModel { alias }) if alias == "number-model"
    ));
}

#[test]
fn wildcard_row_match() {
    let mut scenario = Scenario::new(&[
        r#"{"model_name":"openai/*","litellm_params":{"model":"openai/*"},"model_info":{"id":"wild"}}"#,
    ]);
    scenario.metadata("wild", r#"{"mode":"chat","max_input_tokens":4096}"#);
    scenario.answer("openai/b846-request", "b846-request", "openai");
    let group = group_of(&scenario, "openai/b846-request");
    assert_eq!(group.model_group, "openai/b846-request");
    assert_eq!(group.max_input_tokens, Some(decimal("4096")));
}

#[test]
fn unmatched_row_ignored() {
    let mut scenario = Scenario::new(&[
        r#"{"model_name":"openai/*","litellm_params":{"model":"openai/*"},"model_info":{"id":"wild"}}"#,
        r#"{"model_name":"other","litellm_params":{"model":"openai/b846-other"},"model_info":{"id":"o"}}"#,
    ]);
    scenario.metadata("wild", r#"{"mode":"chat"}"#);
    scenario.metadata("o", r#"{"mode":"chat"}"#);
    scenario.answer("openai/b846-other", "b846-other", "openai");
    assert_eq!(scenario.group("anthropic/claude-b846").expect("info"), None);
}

#[test]
fn fast_mode_all_deployments() {
    let mut scenario = Scenario::new(&[ALPHA, BETA]);
    scenario.metadata("a", r#"{"mode":"chat"}"#);
    scenario.metadata("b", r#"{"mode":"chat"}"#);
    scenario.answer("openai/b846-alpha", "b846-alpha", "openai");
    scenario.answer("anthropic/b846-beta", "claude-b846-fast", "anthropic");
    scenario.fast_mode = fast_mode_catalog(r#"{"claude-b846-fast":{"supports_fast_mode":true}}"#);
    assert!(!group_of(&scenario, "g").supports_fast_mode);

    let mut fast = Scenario::new(&[BETA]);
    fast.metadata("b", r#"{"mode":"chat"}"#);
    fast.answer("anthropic/b846-beta", "claude-b846-fast", "anthropic");
    fast.fast_mode = fast_mode_catalog(r#"{"claude-b846-fast":{"supports_fast_mode":true}}"#);
    assert!(group_of(&fast, "g").supports_fast_mode);

    let mut unknown = Scenario::new(&[BETA]);
    unknown.metadata("b", r#"{"mode":"chat"}"#);
    unknown.answer("anthropic/b846-beta", "claude-b846-unknown", "anthropic");
    unknown.fast_mode = fast_mode_catalog(r#"{"claude-b846-fast":{"supports_fast_mode":true}}"#);
    assert!(!group_of(&unknown, "g").supports_fast_mode);
}

#[test]
fn reasoning_intersection() {
    let mut scenario = Scenario::new(&[ALPHA, BETA]);
    scenario.metadata(
        "a",
        r#"{"mode":"chat","supports_reasoning":true,"supports_low_reasoning_effort":true,"supports_xhigh_reasoning_effort":true}"#,
    );
    scenario.metadata(
        "b",
        r#"{"mode":"chat","supports_reasoning":true,"supports_low_reasoning_effort":true}"#,
    );
    scenario.answer("openai/b846-alpha", "b846-alpha", "openai");
    scenario.answer("anthropic/b846-beta", "b846-beta", "anthropic");
    let group = group_of(&scenario, "g");
    assert_eq!(
        group.supported_reasoning_efforts,
        Some(vec![
            ReasoningEffort::None,
            ReasoningEffort::Minimal,
            ReasoningEffort::Low,
            ReasoningEffort::Medium,
            ReasoningEffort::High,
        ])
    );
}

#[test]
fn reasoning_unknown_sticky() {
    let mut scenario = Scenario::new(&[ALPHA, BETA]);
    scenario.metadata("a", r#"{"supports_system_messages":null}"#);
    scenario.metadata("b", r#"{"mode":"chat","supports_reasoning":true}"#);
    scenario.answer("openai/b846-alpha", "b846-alpha", "openai");
    scenario.answer("anthropic/b846-beta", "b846-beta", "anthropic");
    assert_eq!(group_of(&scenario, "g").supported_reasoning_efforts, None);
}

#[test]
fn reasoning_known_empty() {
    let mut scenario = Scenario::new(&[ALPHA]);
    scenario.metadata("a", r#"{"mode":"chat","supports_reasoning":false}"#);
    scenario.answer("openai/b846-alpha", "b846-alpha", "openai");
    assert_eq!(
        group_of(&scenario, "g").supported_reasoning_efforts,
        Some(Vec::new())
    );
}

mod cost;
