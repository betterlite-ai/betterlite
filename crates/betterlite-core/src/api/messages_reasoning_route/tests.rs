#![cfg(test)]

use std::sync::Mutex;

use betterlite_models::decisions::Json;
use betterlite_models::full_model_metadata::record::ModelInfo;
use betterlite_models::model_metadata_resolution::names::ModelCostMap;
use indexmap::IndexMap;
use rstest::rstest;

use super::seams::{
    BridgeOutcome, BridgePolicy, BridgeRequest, ModelMetadata, ProviderLookup, ProviderResolution,
    ReasoningRouteError, ReasoningSeams, ResolvedTarget,
};
use super::{AutoSummary, CompletionKwargs, normalize_effort, plain_effort, route_thinking};
use crate::responses_bridge::ModelInfoError;

struct Script {
    provider: Option<ResolvedTarget>,
    info: Option<IndexMap<String, Json>>,
    bridge_mode: Option<String>,
    bridge_fails: bool,
    cost: ModelCostMap,
    lookups: Mutex<Vec<Lookup>>,
    info_calls: Mutex<Vec<(Option<String>, Option<String>)>>,
    bridge_calls: Mutex<Vec<BridgeSeen>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Lookup {
    model: Option<String>,
    provider: Option<String>,
    api_base: Option<String>,
    api_key: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct BridgeSeen {
    model: String,
    provider: String,
    web_search_options: Option<Json>,
    tools: Option<Vec<Json>>,
    reasoning_effort: IndexMap<String, Json>,
    api_base: Option<String>,
}

fn empty_cost() -> ModelCostMap {
    ModelCostMap::parse(b"{}").expect("the empty model map parses")
}

fn script() -> Script {
    Script {
        provider: None,
        info: None,
        bridge_mode: None,
        bridge_fails: false,
        cost: empty_cost(),
        lookups: Mutex::new(Vec::new()),
        info_calls: Mutex::new(Vec::new()),
        bridge_calls: Mutex::new(Vec::new()),
    }
}

fn resolve_as(local: &str, provider: &str, api_base: Option<&str>) -> ResolvedTarget {
    ResolvedTarget {
        model: local.to_owned(),
        provider: provider.to_owned(),
        api_base: api_base.map(str::to_owned),
    }
}

fn completion(kwargs: &str) -> CompletionKwargs {
    CompletionKwargs {
        fields: serde_json::from_str(kwargs).expect("the kwargs parse"),
    }
}

fn map(text: &str) -> IndexMap<String, Json> {
    serde_json::from_str(text).expect("the map parses")
}

fn value(text: &str) -> Json {
    serde_json::from_str(text).expect("the value parses")
}

fn seams(script: &Script) -> ReasoningSeams<'_> {
    ReasoningSeams {
        provider: script,
        metadata: script,
        bridge: script,
    }
}

fn locked<T>(slot: &Mutex<Vec<T>>) -> Vec<T>
where
    T: Clone,
{
    slot.lock().expect("the lock is free").clone()
}

impl ProviderResolution for Script {
    fn resolve(&self, lookup: &ProviderLookup<'_>) -> Option<ResolvedTarget> {
        self.lookups.lock().expect("the lock is free").push(Lookup {
            model: lookup.model.map(str::to_owned),
            provider: lookup.custom_llm_provider.map(str::to_owned),
            api_base: lookup.api_base.map(str::to_owned),
            api_key: lookup.api_key.map(str::to_owned),
        });
        self.provider.clone()
    }
}

impl ModelMetadata for Script {
    fn model_info(
        &self,
        model: Option<&str>,
        custom_llm_provider: Option<&str>,
    ) -> Option<ModelInfo> {
        self.info_calls.lock().expect("the lock is free").push((
            model.map(str::to_owned),
            custom_llm_provider.map(str::to_owned),
        ));
        self.info.clone().map(|fields| ModelInfo { fields })
    }

    fn model_cost(&self) -> &ModelCostMap {
        &self.cost
    }
}

impl BridgePolicy for Script {
    fn bridge(&self, request: &BridgeRequest<'_>) -> Result<BridgeOutcome, ModelInfoError> {
        self.bridge_calls
            .lock()
            .expect("the lock is free")
            .push(BridgeSeen {
                model: request.model.to_owned(),
                provider: request.custom_llm_provider.to_owned(),
                web_search_options: request.web_search_options.cloned(),
                tools: request.tools.map(<[Json]>::to_vec),
                reasoning_effort: request.reasoning_effort.clone(),
                api_base: request.api_base.map(str::to_owned),
            });
        if self.bridge_fails {
            return Err(ModelInfoError::Unknown {
                model: request.model.to_owned(),
            });
        }
        Ok(BridgeOutcome {
            mode: self.bridge_mode.clone(),
            model: request.model.to_owned(),
        })
    }
}

#[rstest]
#[case::explicit_openai(
    r#"{"model":"openai/o3","custom_llm_provider":"openai"}"#,
    r#"{"type":"enabled"}"#,
    None,
    None,
    r#"{"model":"openai/responses/o3","custom_llm_provider":"openai"}"#
)]
#[case::bare_model(
    r#"{"model":"o3","custom_llm_provider":"openai"}"#,
    r#"{"type":"enabled"}"#,
    None,
    None,
    r#"{"model":"openai/responses/o3","custom_llm_provider":"openai"}"#
)]
#[case::inferred_provider(
    r#"{"model":"o3"}"#,
    r#"{"type":"enabled"}"#,
    Some("o3"),
    None,
    r#"{"model":"openai/responses/o3"}"#
)]
#[case::inference_fails(
    r#"{"model":"o3"}"#,
    r#"{"type":"enabled"}"#,
    None,
    None,
    r#"{"model":"o3"}"#
)]
#[case::null_provider_infers(
    r#"{"model":"o3","custom_llm_provider":null}"#,
    r#"{"type":"enabled"}"#,
    Some("o3"),
    None,
    r#"{"model":"openai/responses/o3","custom_llm_provider":null}"#
)]
#[case::explicit_provider_wins(
    r#"{"model":"anthropic/claude","custom_llm_provider":"anthropic"}"#,
    r#"{"type":"enabled"}"#,
    Some("o3"),
    None,
    r#"{"model":"anthropic/claude","custom_llm_provider":"anthropic"}"#
)]
#[case::empty_provider_blocks(
    r#"{"model":"o3","custom_llm_provider":""}"#,
    r#"{"type":"enabled"}"#,
    Some("o3"),
    None,
    r#"{"model":"o3","custom_llm_provider":""}"#
)]
#[case::other_provider(
    r#"{"model":"o3","custom_llm_provider":"azure"}"#,
    r#"{"type":"enabled"}"#,
    None,
    None,
    r#"{"model":"o3","custom_llm_provider":"azure"}"#
)]
#[case::non_string_provider(
    r#"{"model":"o3","custom_llm_provider":7}"#,
    r#"{"type":"enabled"}"#,
    Some("o3"),
    None,
    r#"{"model":"o3","custom_llm_provider":7}"#
)]
#[case::already_prefixed(
    r#"{"model":"openai/responses/o3","custom_llm_provider":"openai"}"#,
    r#"{"type":"enabled"}"#,
    None,
    None,
    r#"{"model":"openai/responses/o3","custom_llm_provider":"openai"}"#
)]
#[case::foreign_prefix_stays(
    r#"{"model":"azure/gpt-5","custom_llm_provider":"openai"}"#,
    r#"{"type":"enabled"}"#,
    None,
    None,
    r#"{"model":"openai/responses/azure/gpt-5","custom_llm_provider":"openai"}"#
)]
#[case::empty_model(
    r#"{"model":"","custom_llm_provider":"openai"}"#,
    r#"{"type":"enabled"}"#,
    None,
    None,
    r#"{"model":"","custom_llm_provider":"openai"}"#
)]
#[case::null_model(
    r#"{"model":null,"custom_llm_provider":"openai"}"#,
    r#"{"type":"enabled"}"#,
    None,
    None,
    r#"{"model":null,"custom_llm_provider":"openai"}"#
)]
#[case::missing_model(
    r#"{"custom_llm_provider":"openai"}"#,
    r#"{"type":"enabled"}"#,
    None,
    None,
    r#"{"custom_llm_provider":"openai"}"#
)]
#[case::disabled_thinking(
    r#"{"model":"o3","custom_llm_provider":"openai"}"#,
    r#"{"type":"disabled"}"#,
    None,
    None,
    r#"{"model":"o3","custom_llm_provider":"openai"}"#
)]
#[case::adaptive_thinking(
    r#"{"model":"o3","custom_llm_provider":"openai"}"#,
    r#"{"type":"adaptive"}"#,
    None,
    None,
    r#"{"model":"o3","custom_llm_provider":"openai"}"#
)]
#[case::thinking_without_type(
    r#"{"model":"o3","custom_llm_provider":"openai"}"#,
    r#"{"summary":"concise"}"#,
    None,
    None,
    r#"{"model":"o3","custom_llm_provider":"openai"}"#
)]
#[case::thinking_not_a_dict(
    r#"{"model":"o3","custom_llm_provider":"openai"}"#,
    r#"["enabled"]"#,
    None,
    None,
    r#"{"model":"o3","custom_llm_provider":"openai"}"#
)]
#[case::thinking_null(
    r#"{"model":"o3","custom_llm_provider":"openai"}"#,
    "null",
    None,
    None,
    r#"{"model":"o3","custom_llm_provider":"openai"}"#
)]
#[case::capability_false(
    r#"{"model":"o3","custom_llm_provider":"openai"}"#,
    r#"{"type":"enabled"}"#,
    None,
    Some(r#"{"supports_reasoning":false}"#),
    r#"{"model":"o3","custom_llm_provider":"openai"}"#
)]
#[case::capability_zero_is_not_false(
    r#"{"model":"o3","custom_llm_provider":"openai"}"#,
    r#"{"type":"enabled"}"#,
    None,
    Some(r#"{"supports_reasoning":0}"#),
    r#"{"model":"openai/responses/o3","custom_llm_provider":"openai"}"#
)]
#[case::capability_true(
    r#"{"model":"o3","custom_llm_provider":"openai"}"#,
    r#"{"type":"enabled"}"#,
    None,
    Some(r#"{"supports_reasoning":true}"#),
    r#"{"model":"openai/responses/o3","custom_llm_provider":"openai"}"#
)]
#[case::capability_null(
    r#"{"model":"o3","custom_llm_provider":"openai"}"#,
    r#"{"type":"enabled"}"#,
    None,
    Some(r#"{"supports_reasoning":null}"#),
    r#"{"model":"openai/responses/o3","custom_llm_provider":"openai"}"#
)]
#[case::empty_capability(
    r#"{"model":"o3","custom_llm_provider":"openai"}"#,
    r#"{"type":"enabled"}"#,
    None,
    Some("{}"),
    r#"{"model":"openai/responses/o3","custom_llm_provider":"openai"}"#
)]
#[case::lookup_failure_promotes(
    r#"{"model":"o3","custom_llm_provider":"openai"}"#,
    r#"{"type":"enabled"}"#,
    None,
    None,
    r#"{"model":"openai/responses/o3","custom_llm_provider":"openai"}"#
)]
fn thinking_promotes_only_openai(
    #[case] kwargs: &str,
    #[case] thinking: &str,
    #[case] resolution: Option<&str>,
    #[case] info: Option<&str>,
    #[case] expected: &str,
) {
    let mut script = script();
    script.provider = resolution.map(|local| resolve_as(local, "openai", None));
    script.info = info.map(map);
    let mut kwargs = completion(kwargs);
    let infers = matches!(
        kwargs.fields.get("custom_llm_provider"),
        None | Some(Json::Null)
    );
    let thinking = value(thinking);
    route_thinking(
        &mut kwargs,
        Some(&thinking),
        AutoSummary {
            global: false,
            environment: None,
        },
        &seams(&script),
    );
    assert_eq!(kwargs.fields, map(expected));
    assert_eq!(
        locked(&script.lookups).len(),
        usize::from(infers),
        "only an absent or null provider is inferred"
    );
}

#[rstest]
#[case::string_effort_auto_off(
    r#"{"model":"o3","custom_llm_provider":"openai","reasoning_effort":"high"}"#,
    r#"{"type":"enabled"}"#,
    false,
    None,
    r#"{"model":"openai/responses/o3","custom_llm_provider":"openai","reasoning_effort":{"effort":"high"}}"#
)]
#[case::string_effort_auto_on(
    r#"{"model":"o3","custom_llm_provider":"openai","reasoning_effort":"high"}"#,
    r#"{"type":"enabled"}"#,
    true,
    None,
    r#"{"model":"openai/responses/o3","custom_llm_provider":"openai","reasoning_effort":{"effort":"high","summary":"detailed"}}"#
)]
#[case::environment_turns_it_on(
    r#"{"model":"o3","custom_llm_provider":"openai","reasoning_effort":"high"}"#,
    r#"{"type":"enabled"}"#,
    false,
    Some("true"),
    r#"{"model":"openai/responses/o3","custom_llm_provider":"openai","reasoning_effort":{"effort":"high","summary":"detailed"}}"#
)]
#[case::environment_case_insensitive(
    r#"{"model":"o3","custom_llm_provider":"openai","reasoning_effort":"high"}"#,
    r#"{"type":"enabled"}"#,
    false,
    Some("TRUE"),
    r#"{"model":"openai/responses/o3","custom_llm_provider":"openai","reasoning_effort":{"effort":"high","summary":"detailed"}}"#
)]
#[case::environment_other_value(
    r#"{"model":"o3","custom_llm_provider":"openai","reasoning_effort":"high"}"#,
    r#"{"type":"enabled"}"#,
    false,
    Some("yes"),
    r#"{"model":"openai/responses/o3","custom_llm_provider":"openai","reasoning_effort":{"effort":"high"}}"#
)]
#[case::explicit_summary_wins(
    r#"{"model":"o3","custom_llm_provider":"openai","reasoning_effort":"high"}"#,
    r#"{"type":"enabled","summary":"concise"}"#,
    true,
    None,
    r#"{"model":"openai/responses/o3","custom_llm_provider":"openai","reasoning_effort":{"effort":"high","summary":"concise"}}"#
)]
#[case::dict_auto_off(
    r#"{"model":"o3","custom_llm_provider":"openai","reasoning_effort":{"effort":"high"}}"#,
    r#"{"type":"enabled"}"#,
    false,
    None,
    r#"{"model":"openai/responses/o3","custom_llm_provider":"openai","reasoning_effort":{"effort":"high"}}"#
)]
#[case::dict_auto_on(
    r#"{"model":"o3","custom_llm_provider":"openai","reasoning_effort":{"effort":"high"}}"#,
    r#"{"type":"enabled"}"#,
    true,
    None,
    r#"{"model":"openai/responses/o3","custom_llm_provider":"openai","reasoning_effort":{"effort":"high","summary":"detailed"}}"#
)]
#[case::dict_explicit_summary(
    r#"{"model":"o3","custom_llm_provider":"openai","reasoning_effort":{"effort":"high"}}"#,
    r#"{"type":"enabled","summary":"concise"}"#,
    false,
    None,
    r#"{"model":"openai/responses/o3","custom_llm_provider":"openai","reasoning_effort":{"effort":"high","summary":"concise"}}"#
)]
#[case::dict_summary_null_preserved(
    r#"{"model":"o3","custom_llm_provider":"openai","reasoning_effort":{"effort":"high","summary":null}}"#,
    r#"{"type":"enabled","summary":"concise"}"#,
    true,
    None,
    r#"{"model":"openai/responses/o3","custom_llm_provider":"openai","reasoning_effort":{"effort":"high","summary":null}}"#
)]
#[case::dict_generate_summary_preserved(
    r#"{"model":"o3","custom_llm_provider":"openai","reasoning_effort":{"effort":"high","generate_summary":false}}"#,
    r#"{"type":"enabled","summary":"concise"}"#,
    true,
    None,
    r#"{"model":"openai/responses/o3","custom_llm_provider":"openai","reasoning_effort":{"effort":"high","generate_summary":false}}"#
)]
#[case::dict_falsy_summary_is_replaced(
    r#"{"model":"o3","custom_llm_provider":"openai","reasoning_effort":{"effort":"high"}}"#,
    r#"{"type":"enabled","summary":""}"#,
    true,
    None,
    r#"{"model":"openai/responses/o3","custom_llm_provider":"openai","reasoning_effort":{"effort":"high","summary":"detailed"}}"#
)]
#[case::dict_zero_summary_is_replaced(
    r#"{"model":"o3","custom_llm_provider":"openai","reasoning_effort":{"effort":"high"}}"#,
    r#"{"type":"enabled","summary":0}"#,
    true,
    None,
    r#"{"model":"openai/responses/o3","custom_llm_provider":"openai","reasoning_effort":{"effort":"high","summary":"detailed"}}"#
)]
#[case::dict_number_summary_is_used(
    r#"{"model":"o3","custom_llm_provider":"openai","reasoning_effort":{"effort":"high"}}"#,
    r#"{"type":"enabled","summary":7}"#,
    false,
    None,
    r#"{"model":"openai/responses/o3","custom_llm_provider":"openai","reasoning_effort":{"effort":"high","summary":7}}"#
)]
#[case::empty_effort_unchanged(
    r#"{"model":"o3","custom_llm_provider":"openai","reasoning_effort":""}"#,
    r#"{"type":"enabled"}"#,
    true,
    None,
    r#"{"model":"openai/responses/o3","custom_llm_provider":"openai","reasoning_effort":""}"#
)]
#[case::null_effort_unchanged(
    r#"{"model":"o3","custom_llm_provider":"openai","reasoning_effort":null}"#,
    r#"{"type":"enabled"}"#,
    true,
    None,
    r#"{"model":"openai/responses/o3","custom_llm_provider":"openai","reasoning_effort":null}"#
)]
#[case::prefixed_model_still_converts(
    r#"{"model":"openai/responses/o3","custom_llm_provider":"openai","reasoning_effort":"high"}"#,
    r#"{"type":"enabled"}"#,
    true,
    None,
    r#"{"model":"openai/responses/o3","custom_llm_provider":"openai","reasoning_effort":{"effort":"high","summary":"detailed"}}"#
)]
fn summary_options_preserve_presence(
    #[case] kwargs: &str,
    #[case] thinking: &str,
    #[case] global: bool,
    #[case] environment: Option<&str>,
    #[case] expected: &str,
) {
    let script = script();
    let mut kwargs = completion(kwargs);
    let thinking = value(thinking);
    route_thinking(
        &mut kwargs,
        Some(&thinking),
        AutoSummary {
            global,
            environment,
        },
        &seams(&script),
    );
    assert_eq!(kwargs.fields, map(expected));
}

#[test]
fn refused_capabilities_leave_the_effort_alone() {
    let mut script = script();
    script.info = Some(map(r#"{"supports_reasoning":false}"#));
    let mut kwargs =
        completion(r#"{"model":"o3","custom_llm_provider":"openai","reasoning_effort":"high"}"#);
    let thinking = value(r#"{"type":"enabled","summary":"concise"}"#);
    route_thinking(
        &mut kwargs,
        Some(&thinking),
        AutoSummary {
            global: true,
            environment: None,
        },
        &seams(&script),
    );
    assert_eq!(
        kwargs.fields,
        map(r#"{"model":"o3","custom_llm_provider":"openai","reasoning_effort":"high"}"#)
    );
}

#[test]
fn disabled_thinking_skips_the_capability_lookup() {
    let script = script();
    let mut kwargs = completion(r#"{"model":"o3","custom_llm_provider":"openai"}"#);
    let thinking = value(r#"{"type":"disabled"}"#);
    route_thinking(
        &mut kwargs,
        Some(&thinking),
        AutoSummary {
            global: false,
            environment: None,
        },
        &seams(&script),
    );
    assert!(locked(&script.info_calls).is_empty());
}

#[test]
fn route_reads_capabilities_before_rewriting_the_model() {
    let mut script = script();
    script.info = Some(map(r#"{"supports_reasoning":true}"#));
    let mut kwargs = completion(r#"{"model":"openai/o3","custom_llm_provider":"openai"}"#);
    let thinking = value(r#"{"type":"enabled"}"#);
    route_thinking(
        &mut kwargs,
        Some(&thinking),
        AutoSummary {
            global: false,
            environment: None,
        },
        &seams(&script),
    );
    assert_eq!(
        locked(&script.info_calls),
        vec![(Some("openai/o3".to_owned()), Some("openai".to_owned()))]
    );
}

#[rstest]
#[case::string_max_to_high(
    r#"{"model":"deployment-a","custom_llm_provider":"openai","reasoning_effort":"max"}"#,
    Some(r#"{"supports_reasoning":true,"reasoning_effort_levels":["high"]}"#),
    Some(("deployment-a", Some("openai"))),
    r#"{"model":"deployment-a","custom_llm_provider":"openai","reasoning_effort":"high"}"#
)]
#[case::dict_max_keeps_the_other_fields(
    r#"{"model":"deployment-a","custom_llm_provider":"openai","reasoning_effort":{"effort":"max","summary":"concise","extra":1}}"#,
    Some(r#"{"supports_reasoning":true,"reasoning_effort_levels":["high"]}"#),
    Some(("deployment-a", Some("openai"))),
    r#"{"model":"deployment-a","custom_llm_provider":"openai","reasoning_effort":{"effort":"high","summary":"concise","extra":1}}"#
)]
#[case::xhigh_to_high(
    r#"{"model":"deployment-a","custom_llm_provider":"openai","reasoning_effort":"xhigh"}"#,
    Some(r#"{"supports_reasoning":true,"reasoning_effort_levels":["high"]}"#),
    Some(("deployment-a", Some("openai"))),
    r#"{"model":"deployment-a","custom_llm_provider":"openai","reasoning_effort":"high"}"#
)]
#[case::minimal_to_low(
    r#"{"model":"deployment-a","custom_llm_provider":"openai","reasoning_effort":"minimal"}"#,
    Some(r#"{"supports_reasoning":true,"reasoning_effort_levels":["low"]}"#),
    Some(("deployment-a", Some("openai"))),
    r#"{"model":"deployment-a","custom_llm_provider":"openai","reasoning_effort":"low"}"#
)]
#[case::max_without_metadata_keeps_the_floor(
    r#"{"model":"deployment-a","custom_llm_provider":"openai","reasoning_effort":"max"}"#,
    None,
    Some(("deployment-a", Some("openai"))),
    r#"{"model":"deployment-a","custom_llm_provider":"openai","reasoning_effort":"high"}"#
)]
#[case::max_with_empty_metadata_keeps_the_floor(
    r#"{"model":"deployment-a","custom_llm_provider":"openai","reasoning_effort":"max"}"#,
    Some("{}"),
    Some(("deployment-a", Some("openai"))),
    r#"{"model":"deployment-a","custom_llm_provider":"openai","reasoning_effort":"high"}"#
)]
#[case::absent_model_looks_up_the_empty_name(
    r#"{"reasoning_effort":"max"}"#,
    None,
    Some(("", None)),
    r#"{"reasoning_effort":"high"}"#
)]
#[case::absent_provider_looks_up_no_provider(
    r#"{"model":"deployment-a","reasoning_effort":"max"}"#,
    Some(r#"{"supports_reasoning":true,"reasoning_effort_levels":["high"]}"#),
    Some(("deployment-a", None)),
    r#"{"model":"deployment-a","reasoning_effort":"high"}"#
)]
#[case::already_routed_model_is_read_as_written(
    r#"{"model":"openai/responses/o3","custom_llm_provider":"openai","reasoning_effort":"max"}"#,
    Some(r#"{"supports_reasoning":true,"reasoning_effort_levels":["high"]}"#),
    Some(("openai/responses/o3", Some("openai"))),
    r#"{"model":"openai/responses/o3","custom_llm_provider":"openai","reasoning_effort":"high"}"#
)]
#[case::high_is_never_looked_up(
    r#"{"model":"deployment-a","custom_llm_provider":"openai","reasoning_effort":"high"}"#,
    Some(r#"{"supports_reasoning":true,"reasoning_effort_levels":["low"]}"#),
    None,
    r#"{"model":"deployment-a","custom_llm_provider":"openai","reasoning_effort":"high"}"#
)]
#[case::bogus_is_never_looked_up(
    r#"{"model":"deployment-a","custom_llm_provider":"openai","reasoning_effort":"bogus"}"#,
    None,
    None,
    r#"{"model":"deployment-a","custom_llm_provider":"openai","reasoning_effort":"bogus"}"#
)]
#[case::empty_effort_unchanged(
    r#"{"model":"deployment-a","custom_llm_provider":"openai","reasoning_effort":""}"#,
    None,
    None,
    r#"{"model":"deployment-a","custom_llm_provider":"openai","reasoning_effort":""}"#
)]
#[case::null_effort_unchanged(
    r#"{"model":"deployment-a","custom_llm_provider":"openai","reasoning_effort":null}"#,
    None,
    None,
    r#"{"model":"deployment-a","custom_llm_provider":"openai","reasoning_effort":null}"#
)]
#[case::number_effort_unchanged(
    r#"{"model":"deployment-a","custom_llm_provider":"openai","reasoning_effort":5}"#,
    None,
    None,
    r#"{"model":"deployment-a","custom_llm_provider":"openai","reasoning_effort":5}"#
)]
#[case::list_effort_unchanged(
    r#"{"model":"deployment-a","custom_llm_provider":"openai","reasoning_effort":["max"]}"#,
    None,
    None,
    r#"{"model":"deployment-a","custom_llm_provider":"openai","reasoning_effort":["max"]}"#
)]
#[case::dict_without_effort_unchanged(
    r#"{"model":"deployment-a","custom_llm_provider":"openai","reasoning_effort":{"summary":"concise"}}"#,
    None,
    None,
    r#"{"model":"deployment-a","custom_llm_provider":"openai","reasoning_effort":{"summary":"concise"}}"#
)]
#[case::non_string_effort_in_dict_unchanged(
    r#"{"model":"deployment-a","custom_llm_provider":"openai","reasoning_effort":{"effort":7}}"#,
    Some(r#"{"supports_reasoning":true,"reasoning_effort_levels":["high"]}"#),
    None,
    r#"{"model":"deployment-a","custom_llm_provider":"openai","reasoning_effort":{"effort":7}}"#
)]
fn normalize_before_route(
    #[case] kwargs: &str,
    #[case] info: Option<&str>,
    #[case] lookup: Option<(&str, Option<&str>)>,
    #[case] expected: &str,
) {
    let mut script = script();
    script.info = info.map(map);
    let mut kwargs = completion(kwargs);
    normalize_effort(&mut kwargs, &script);
    assert_eq!(kwargs.fields, map(expected));
    let calls = locked(&script.info_calls);
    match lookup {
        Some((model, provider)) => assert_eq!(
            calls,
            vec![(Some(model.to_owned()), provider.map(str::to_owned))]
        ),
        None => assert!(calls.is_empty()),
    }
}

#[test]
fn normalize_reads_a_null_model_as_no_model() {
    let mut script = script();
    script.info = Some(map(r#"{"supports_reasoning":true}"#));
    let mut kwargs = completion(r#"{"model":null,"reasoning_effort":"max"}"#);
    normalize_effort(&mut kwargs, &script);
    assert_eq!(
        kwargs.fields,
        map(r#"{"model":null,"reasoning_effort":"high"}"#)
    );
    assert_eq!(locked(&script.info_calls), vec![(None, None)]);
}

#[test]
fn normalize_then_route_looks_up_the_original_model() {
    let mut script = script();
    script.info = Some(map(
        r#"{"supports_reasoning":true,"reasoning_effort_levels":["high"]}"#,
    ));
    let mut kwargs =
        completion(r#"{"model":"o3","custom_llm_provider":"openai","reasoning_effort":"max"}"#);
    normalize_effort(&mut kwargs, &script);
    let thinking = value(r#"{"type":"enabled"}"#);
    route_thinking(
        &mut kwargs,
        Some(&thinking),
        AutoSummary {
            global: false,
            environment: None,
        },
        &seams(&script),
    );
    assert_eq!(
        kwargs.fields,
        map(
            r#"{"model":"openai/responses/o3","custom_llm_provider":"openai","reasoning_effort":{"effort":"high"}}"#
        )
    );
    assert_eq!(
        locked(&script.info_calls),
        vec![
            (Some("o3".to_owned()), Some("openai".to_owned())),
            (Some("o3".to_owned()), Some("openai".to_owned())),
        ]
    );
}

#[rstest]
#[case::chat_mode_keeps_the_effort(
    r#"{"model":"o3","custom_llm_provider":"openai","reasoning_effort":{"effort":"high","summary":null}}"#,
    r#"{"type":"enabled"}"#,
    Some(("o3", "openai")),
    Some("chat"),
    Some("high")
)]
#[case::absent_mode_keeps_the_effort(
    r#"{"model":"o3","custom_llm_provider":"openai","reasoning_effort":{"effort":"high","summary":null}}"#,
    r#"{"type":"enabled"}"#,
    Some(("o3", "openai")),
    None,
    Some("high")
)]
#[case::responses_mode_returns_none(
    r#"{"model":"o3","custom_llm_provider":"openai","reasoning_effort":{"effort":"high","summary":null}}"#,
    r#"{"type":"enabled"}"#,
    Some(("o3", "openai")),
    Some("responses"),
    None
)]
#[case::empty_effort_is_still_an_effort(
    r#"{"model":"o3","custom_llm_provider":"openai","reasoning_effort":{"effort":"","summary":null}}"#,
    r#"{"type":"enabled"}"#,
    Some(("o3", "openai")),
    Some("chat"),
    Some("")
)]
#[case::resolution_failure_returns_none(
    r#"{"model":"o3","custom_llm_provider":"openai","reasoning_effort":{"effort":"high","summary":null}}"#,
    r#"{"type":"enabled"}"#,
    None,
    Some("responses"),
    None
)]
#[case::proxy_provider_returns_none(
    r#"{"model":"o3","custom_llm_provider":"openai","reasoning_effort":{"effort":"high","summary":null}}"#,
    r#"{"type":"enabled"}"#,
    Some(("o3", "litellm_proxy")),
    Some("responses"),
    None
)]
fn plain_effort_requires_chat(
    #[case] kwargs: &str,
    #[case] thinking: &str,
    #[case] provider: Option<(&str, &str)>,
    #[case] bridge: Option<&str>,
    #[case] expected: Option<&str>,
) {
    let mut script = script();
    script.provider = provider.map(|(local, provider)| resolve_as(local, provider, None));
    script.bridge_mode = bridge.map(str::to_owned);
    let kwargs = completion(kwargs);
    let thinking = value(thinking);
    let effort =
        plain_effort(&kwargs, Some(&thinking), &seams(&script)).expect("the bridge policy answers");
    assert_eq!(effort.as_deref(), expected);
}

#[rstest]
#[case::no_reasoning_effort(r#"{"model":"o3"}"#, r#"{"type":"enabled"}"#)]
#[case::string_effort(r#"{"model":"o3","reasoning_effort":"high"}"#, r#"{"type":"enabled"}"#)]
#[case::list_effort(
    r#"{"model":"o3","reasoning_effort":["high"]}"#,
    r#"{"type":"enabled"}"#
)]
#[case::summary_missing(
    r#"{"model":"o3","reasoning_effort":{"effort":"high"}}"#,
    r#"{"type":"enabled"}"#
)]
#[case::effort_missing(
    r#"{"model":"o3","reasoning_effort":{"summary":null}}"#,
    r#"{"type":"enabled"}"#
)]
#[case::effort_not_a_string(
    r#"{"model":"o3","reasoning_effort":{"effort":5,"summary":null}}"#,
    r#"{"type":"enabled"}"#
)]
#[case::empty_model(
    r#"{"model":"","reasoning_effort":{"effort":"high","summary":null}}"#,
    r#"{"type":"enabled"}"#
)]
#[case::null_model(
    r#"{"model":null,"reasoning_effort":{"effort":"high","summary":null}}"#,
    r#"{"type":"enabled"}"#
)]
#[case::thinking_null(
    r#"{"model":"o3","reasoning_effort":{"effort":"high","summary":null}}"#,
    "null"
)]
#[case::thinking_empty(
    r#"{"model":"o3","reasoning_effort":{"effort":"high","summary":null}}"#,
    "{}"
)]
#[case::thinking_empty_list(
    r#"{"model":"o3","reasoning_effort":{"effort":"high","summary":null}}"#,
    "[]"
)]
#[case::thinking_zero(
    r#"{"model":"o3","reasoning_effort":{"effort":"high","summary":null}}"#,
    "0"
)]
#[case::thinking_empty_string(
    r#"{"model":"o3","reasoning_effort":{"effort":"high","summary":null}}"#,
    r#""""#
)]
fn plain_effort_returns_none_without_resolving(#[case] kwargs: &str, #[case] thinking: &str) {
    let mut script = script();
    script.provider = Some(resolve_as("o3", "openai", None));
    script.bridge_mode = Some("chat".to_owned());
    let kwargs = completion(kwargs);
    let thinking = value(thinking);
    let effort = plain_effort(&kwargs, Some(&thinking), &seams(&script))
        .expect("no bridge policy is consulted");
    assert_eq!(effort, None);
    assert!(locked(&script.lookups).is_empty());
    assert!(locked(&script.bridge_calls).is_empty());
}

#[test]
fn plain_effort_accepts_any_truthy_thinking_value() {
    let mut script = script();
    script.provider = Some(resolve_as("o3", "openai", None));
    script.bridge_mode = Some("chat".to_owned());
    let kwargs =
        completion(r#"{"model":"o3","reasoning_effort":{"effort":"high","summary":null}}"#);
    let thinking = value(r#""enabled""#);
    let effort =
        plain_effort(&kwargs, Some(&thinking), &seams(&script)).expect("the policy answers");
    assert_eq!(effort.as_deref(), Some("high"));
}

#[test]
fn plain_effort_treats_non_string_lookup_values_as_absent() {
    let mut script = script();
    script.provider = Some(resolve_as("o3", "openai", None));
    script.bridge_mode = Some("chat".to_owned());
    let kwargs = completion(
        r#"{"model":"o3","custom_llm_provider":7,"api_base":[],"api_key":{},"reasoning_effort":{"effort":"high","summary":null}}"#,
    );
    let thinking = value(r#"{"type":"enabled"}"#);
    let effort =
        plain_effort(&kwargs, Some(&thinking), &seams(&script)).expect("the bridge policy answers");
    assert_eq!(effort.as_deref(), Some("high"));
    assert_eq!(
        locked(&script.lookups),
        vec![Lookup {
            model: Some("o3".to_owned()),
            provider: None,
            api_base: None,
            api_key: None,
        }]
    );
}

#[rstest]
#[case::only_objects_and_lists_pass(
    r#"{"model":"o3","custom_llm_provider":"openai","web_search_options":{"search_context_size":"high"},"tools":[{"type":"function"}],"reasoning_effort":{"effort":"high","summary":"concise","extra":[1]}}"#,
    Some(r#"{"search_context_size":"high"}"#),
    Some(r#"[{"type":"function"}]"#)
)]
#[case::other_shapes_are_dropped(
    r#"{"model":"o3","custom_llm_provider":"openai","web_search_options":"high","tools":{"type":"function"},"reasoning_effort":{"effort":"high","summary":null}}"#,
    None,
    None
)]
fn plain_effort_passes_the_resolved_target_to_the_bridge(
    #[case] kwargs: &str,
    #[case] web_search_options: Option<&str>,
    #[case] tools: Option<&str>,
) {
    let mut script = script();
    script.provider = Some(resolve_as("local-o3", "openai", Some("https://api.test")));
    script.bridge_mode = Some("chat".to_owned());
    let kwargs = completion(kwargs);
    let thinking = value(r#"{"type":"enabled"}"#);
    let effort =
        plain_effort(&kwargs, Some(&thinking), &seams(&script)).expect("the bridge policy answers");
    assert_eq!(effort.as_deref(), Some("high"));
    let reasoning_effort = match &kwargs.fields["reasoning_effort"] {
        Json::Object(fields) => fields.clone(),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            panic!("the case carries a reasoning dict")
        }
    };
    assert_eq!(
        locked(&script.bridge_calls),
        vec![BridgeSeen {
            model: "local-o3".to_owned(),
            provider: "openai".to_owned(),
            web_search_options: web_search_options.map(value),
            tools: tools.map(|tools| match value(tools) {
                Json::List(items) => items,
                Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_) =>
                    panic!("the case carries a tools list"),
            }),
            reasoning_effort,
            api_base: Some("https://api.test".to_owned()),
        }]
    );
}

#[test]
fn plain_effort_propagates_the_bridge_failure() {
    let mut script = script();
    script.provider = Some(resolve_as("o3", "openai", None));
    script.bridge_fails = true;
    let kwargs =
        completion(r#"{"model":"o3","reasoning_effort":{"effort":"high","summary":null}}"#);
    let thinking = value(r#"{"type":"enabled"}"#);
    let error = plain_effort(&kwargs, Some(&thinking), &seams(&script))
        .expect_err("the policy failure is not swallowed");
    assert!(matches!(error, ReasoningRouteError::Bridge { .. }));
}
