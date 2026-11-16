#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_core::shadow_evidence::cost::PriceMap;
use betterlite_core::shadow_evidence::records::{CallFailure, JudgeVerdict};
use betterlite_models::catalog::{ModelCatalog, UnknownFields};
use betterlite_models::decisions::Json;
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::judge_primitives::provider_qualified::ProviderCache;
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::resolution::ResolutionContext;
use indexmap::IndexMap;
use rstest::rstest;
use rust_decimal::Decimal;
use serde_json::Number;

use super::{call_judge, conversation_text, forwarded_team_id, judge_call_failure};
use crate::judge_targets::call::JudgeCall;
use crate::judge_targets::error::{JudgeCallError, JudgeDispatchError};
use crate::judge_targets::router::JudgeRouter;
use crate::judge_targets::sdk::JudgeSdk;
use crate::shadow_calls::judge_prompt::{PAIRWISE_JUDGE_SYSTEM_PROMPT, judge_user_prompt};
use crate::shadow_calls::response_format::pairwise_judge_response_format;
use crate::shadow_calls::{
    JudgeDeps, JudgeRequest, PositionCoin, ShadowRouter, ShadowRouterProvider,
};

const CATALOG: &str = r#"{"gpt-4o": {"litellm_provider": "openai"}}"#;

struct Harness {
    catalog: ModelCatalog,
    known: BTreeSet<String>,
    environment: BTreeMap<String, String>,
    custom: BTreeSet<String>,
    cache: ProviderCache,
}

impl Harness {
    fn new() -> Self {
        Self {
            catalog: ModelCatalog::parse(CATALOG.as_bytes(), UnknownFields::Reject).unwrap(),
            known: BTreeSet::new(),
            environment: BTreeMap::new(),
            custom: BTreeSet::new(),
            cache: ProviderCache::default(),
        }
    }

    fn context(&self) -> ResolutionContext<'_> {
        ResolutionContext {
            catalog: &self.catalog,
            known_model_names: &self.known,
            custom_providers: &self.custom,
            environment: &self.environment,
            registry: JSONProviderRegistry::default(),
            policies: ProviderPolicies::default(),
            globals: GlobalCredentials::default(),
        }
    }
}

struct Recorder {
    reply: Option<Json>,
    calls: Mutex<Vec<JudgeCall>>,
}

impl Recorder {
    fn answer(&self, call: JudgeCall) -> Result<Json, JudgeDispatchError> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(call);
        match &self.reply {
            Some(response) => Ok(response.clone()),
            None => Err(JudgeDispatchError {
                message: "judge provider exploded".to_owned(),
            }),
        }
    }

    fn calls(&self) -> Vec<JudgeCall> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

struct RouterFake {
    served: Vec<String>,
    teams: Mutex<Vec<Option<String>>>,
    recorder: Arc<Recorder>,
}

impl RouterFake {
    fn teams(&self) -> Vec<Option<String>> {
        self.teams
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[async_trait]
impl JudgeRouter for RouterFake {
    fn resolved_litellm_models(&self, _model: &str, team_id: Option<&str>) -> Vec<String> {
        self.teams
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(team_id.map(str::to_owned));
        self.served.clone()
    }

    async fn acompletion(&self, call: JudgeCall) -> Result<Json, JudgeDispatchError> {
        self.recorder.answer(call)
    }
}

struct SdkFake {
    recorder: Arc<Recorder>,
}

#[async_trait]
impl JudgeSdk for SdkFake {
    async fn acompletion(&self, call: JudgeCall) -> Result<Json, JudgeDispatchError> {
        self.recorder.answer(call)
    }
}

struct Provider {
    judge: Option<Arc<dyn JudgeRouter>>,
}

impl ShadowRouterProvider for Provider {
    fn shadow_router(&self) -> Option<Arc<dyn ShadowRouter>> {
        None
    }

    fn judge_router(&self) -> Option<Arc<dyn JudgeRouter>> {
        self.judge.clone()
    }
}

struct Coin(bool);

impl PositionCoin for Coin {
    fn real_is_a(&self) -> bool {
        self.0
    }
}

struct Prices(Decimal);

impl PriceMap for Prices {
    fn completion_cost(&self, _response: &Json) -> Option<Decimal> {
        Some(self.0)
    }
}

struct Run {
    verdict: Result<JudgeVerdict, CallFailure>,
    calls: Vec<JudgeCall>,
    teams: Vec<Option<String>>,
}

struct Case {
    reply: Option<Json>,
    router: bool,
    coin: bool,
    model: &'static str,
    messages: Vec<Json>,
    tools: Json,
    parent_metadata: IndexMap<String, Json>,
}

impl Case {
    fn new(reply: Option<Json>) -> Self {
        Self {
            reply,
            router: true,
            coin: true,
            model: "gpt-4o",
            messages: Vec::new(),
            tools: Json::Null,
            parent_metadata: IndexMap::new(),
        }
    }

    fn with_router(self, router: bool) -> Self {
        Self { router, ..self }
    }

    fn with_coin(self, coin: bool) -> Self {
        Self { coin, ..self }
    }

    fn with_model(self, model: &'static str) -> Self {
        Self { model, ..self }
    }

    fn with_messages(self, messages: Vec<Json>) -> Self {
        Self { messages, ..self }
    }

    fn with_tools(self, tools: Json) -> Self {
        Self { tools, ..self }
    }

    fn with_metadata(self, parent_metadata: IndexMap<String, Json>) -> Self {
        Self {
            parent_metadata,
            ..self
        }
    }
}

fn erased(router: &Arc<RouterFake>) -> Arc<dyn JudgeRouter> {
    router.clone()
}

struct Parts {
    harness: Harness,
    recorder: Arc<Recorder>,
    router: Arc<RouterFake>,
    provider: Provider,
    coin: Coin,
    sdk: SdkFake,
    prices: Prices,
}

fn parts(case: &Case) -> Parts {
    let recorder = Arc::new(Recorder {
        reply: case.reply.clone(),
        calls: Mutex::new(Vec::new()),
    });
    let router = Arc::new(RouterFake {
        served: vec![case.model.to_owned()],
        teams: Mutex::new(Vec::new()),
        recorder: recorder.clone(),
    });
    let sdk = SdkFake {
        recorder: recorder.clone(),
    };
    Parts {
        harness: Harness::new(),
        recorder,
        provider: Provider {
            judge: case.router.then(|| erased(&router)),
        },
        coin: Coin(case.coin),
        sdk,
        prices: Prices(Decimal::ZERO),
        router,
    }
}

async fn run_case(case: &Case) -> Run {
    let case_parts = parts(case);
    let context = case_parts.harness.context();
    let deps = JudgeDeps {
        provider: &case_parts.provider,
        coin: &case_parts.coin,
        sdk: &case_parts.sdk,
        cache: &case_parts.harness.cache,
        context: &context,
        prices: &case_parts.prices,
    };
    let request = JudgeRequest {
        judge_model: case.model,
        messages: &case.messages,
        real_text: "real reply",
        shadow_text: "shadow reply",
        tools: &case.tools,
        parent_metadata: &case.parent_metadata,
    };
    let outcome = call_judge(&request, &deps).await;
    Run {
        verdict: outcome,
        calls: case_parts.recorder.calls(),
        teams: case_parts.router.teams(),
    }
}

fn object<const SIZE: usize>(entries: [(&str, Json); SIZE]) -> Json {
    Json::Object(
        entries
            .into_iter()
            .map(|(key, value)| (key.to_owned(), value))
            .collect(),
    )
}

fn number(source: &str) -> Json {
    serde_json::from_str(source).unwrap()
}

fn judge_response(content: Json) -> Json {
    Json::Object(IndexMap::from([
        ("model".to_owned(), Json::Text("judge-model".to_owned())),
        (
            "choices".to_owned(),
            Json::List(vec![Json::Object(IndexMap::from([
                ("message".to_owned(), object([("content", content)])),
                ("finish_reason".to_owned(), Json::Text("stop".to_owned())),
            ]))]),
        ),
    ]))
}

fn verdict(preference: &str, confidence: &str) -> Json {
    judge_response(Json::Text(format!(
        "{{\"preference\": \"{preference}\", \"confidence\": {confidence}}}"
    )))
}

fn stamped(mut response: Json, cost: &str) -> Json {
    if let Json::Object(fields) = &mut response {
        fields.insert(
            "_hidden_params".to_owned(),
            object([("response_cost", number(cost))]),
        );
    }
    response
}

fn chat_message(role: &str, content: &str) -> Json {
    object([
        ("role", Json::Text(role.to_owned())),
        ("content", Json::Text(content.to_owned())),
    ])
}

fn bound_prompt(conversation: &str) -> String {
    judge_user_prompt(conversation, "real reply", "shadow reply", "")
}

#[tokio::test]
async fn the_judge_call_sends_the_system_and_user_messages_and_the_fixed_params() {
    let run = run_case(&Case::new(Some(verdict("A", "0.9")))).await;
    let call = run.calls.first().unwrap();

    assert_eq!(call.model, "gpt-4o");
    assert_eq!(call.num_retries, 0);
    assert_eq!(call.fallbacks, Some(Vec::new()));
    assert!(call.drop_params);
    assert_eq!(
        call.messages,
        vec![
            chat_message("system", PAIRWISE_JUDGE_SYSTEM_PROMPT),
            chat_message("user", &bound_prompt("")),
        ]
    );
    assert_eq!(
        call.params.get("temperature"),
        Some(&Json::Number(Number::from(0_u8)))
    );
    assert_eq!(
        call.params.get("max_tokens"),
        Some(&Json::Number(Number::from(4096_u64)))
    );
    assert_eq!(
        call.params.get("response_format"),
        Some(&pairwise_judge_response_format())
    );
    assert!(call.params.contains_key("metadata"));
}

#[tokio::test]
async fn the_judge_metadata_carries_the_judge_origin_and_asks_under_the_forwarded_team() {
    let metadata = IndexMap::from([
        (
            "user_api_key".to_owned(),
            Json::Text("sk-shadowed".to_owned()),
        ),
        (
            "user_api_key_team_id".to_owned(),
            Json::Text("team-1".to_owned()),
        ),
        (
            "routing_decision".to_owned(),
            Json::Text("dropped".to_owned()),
        ),
    ]);

    let run = run_case(&Case::new(Some(verdict("A", "0.9"))).with_metadata(metadata)).await;

    let expected = object([
        ("user_api_key", Json::Text("sk-shadowed".to_owned())),
        ("user_api_key_team_id", Json::Text("team-1".to_owned())),
        (
            "internal_call_origin",
            Json::Text("shadow_eval_judge".to_owned()),
        ),
    ]);
    assert_eq!(
        run.calls.first().unwrap().params.get("metadata"),
        Some(&expected)
    );
    assert_eq!(run.teams, vec![Some("team-1".to_owned())]);
}

#[tokio::test]
async fn a_judge_call_without_a_team_asks_the_router_without_one() {
    let metadata = IndexMap::from([("user_api_key_team_id".to_owned(), Json::Text("".to_owned()))]);

    let run = run_case(&Case::new(Some(verdict("A", "0.9"))).with_metadata(metadata)).await;

    assert_eq!(run.teams, vec![None]);
}

#[rstest]
#[case::real_first_a(true, "A", "real")]
#[case::real_first_b(true, "B", "shadow")]
#[case::shadow_first_a(false, "A", "shadow")]
#[case::shadow_first_b(false, "B", "real")]
#[tokio::test]
async fn the_verdict_is_unmasked_for_both_coin_results(
    #[case] coin: bool,
    #[case] preference: &str,
    #[case] expected: &str,
) {
    let run = run_case(&Case::new(Some(verdict(preference, "0.5"))).with_coin(coin)).await;

    assert_eq!(run.verdict.unwrap().preference, expected);
}

#[tokio::test]
async fn the_judge_sees_the_shadow_reply_as_a_when_the_coin_says_shadow_first() {
    let run = run_case(&Case::new(Some(verdict("A", "0.9"))).with_coin(false)).await;

    assert_eq!(
        run.calls.first().unwrap().messages.get(1),
        Some(&chat_message(
            "user",
            &judge_user_prompt("", "shadow reply", "real reply", "")
        ))
    );
}

fn multimodal_message() -> Json {
    object([
        ("role", Json::Text("tool".to_owned())),
        (
            "content",
            Json::List(vec![
                object([
                    ("type", Json::Text("text".to_owned())),
                    ("text", Json::Text("part one".to_owned())),
                ]),
                object([
                    ("type", Json::Text("text".to_owned())),
                    ("text", Json::Text("part two".to_owned())),
                ]),
            ]),
        ),
    ])
}

fn conversation_messages() -> Vec<Json> {
    vec![
        object([
            ("role", Json::Text("user".to_owned())),
            ("content", Json::Text("hello".to_owned())),
        ]),
        object([
            ("role", Json::Text("assistant".to_owned())),
            ("content", Json::Null),
        ]),
        object([("content", Json::Text("no role".to_owned()))]),
        object([
            ("role", Json::Null),
            ("content", Json::Text("null role".to_owned())),
        ]),
        object([
            ("role", Json::Text("system".to_owned())),
            ("content", Json::Text("".to_owned())),
        ]),
        multimodal_message(),
    ]
}

#[tokio::test]
async fn the_conversation_text_skips_null_content_and_uppercases_the_roles() {
    let case = Case::new(Some(verdict("A", "0.9"))).with_messages(conversation_messages());

    let run = run_case(&case).await;

    let conversation =
        "USER: hello\nUSER: no role\nNONE: null role\nSYSTEM: \nTOOL: part one part two";
    assert_eq!(
        run.calls.first().unwrap().messages.get(1),
        Some(&chat_message("user", &bound_prompt(conversation)))
    );
}

#[tokio::test]
async fn the_judge_prompt_carries_the_tool_definitions() {
    let tools = serde_json::from_str(
        r#"[{"function": {"name": "get_weather", "description": "Get the weather"}}]"#,
    )
    .unwrap();

    let run = run_case(&Case::new(Some(verdict("A", "0.9"))).with_tools(tools)).await;

    let tool_text = "Tools available to both responses:\n- get_weather: Get the weather";
    assert_eq!(
        run.calls.first().unwrap().messages.get(1),
        Some(&chat_message(
            "user",
            &judge_user_prompt("", "real reply", "shadow reply", tool_text)
        ))
    );
}

#[rstest]
#[case::above_the_range("1.5", "1")]
#[case::below_the_range("-0.2", "0")]
#[case::inside_the_range("0.4", "0.4")]
#[tokio::test]
async fn confidence_outside_zero_and_one_is_clamped(#[case] given: &str, #[case] expected: &str) {
    let run = run_case(&Case::new(Some(verdict("tie", given)))).await;

    assert_eq!(
        run.verdict.unwrap().confidence,
        expected.parse::<Decimal>().unwrap()
    );
}

#[tokio::test]
async fn a_parsed_verdict_carries_the_billed_judge_cost() {
    let run = run_case(&Case::new(Some(stamped(verdict("A", "0.9"), "0.3")))).await;

    let parsed = run.verdict.unwrap();
    assert_eq!(parsed.preference, "real");
    assert_eq!(parsed.cost, Decimal::new(3, 1));
}

#[tokio::test]
async fn a_verdict_with_extra_keys_still_parses() {
    let content = r#"{"preference": "B", "confidence": 0.5, "reason": "B is shorter"}"#;

    let run = run_case(&Case::new(Some(judge_response(Json::Text(
        content.to_owned(),
    )))))
    .await;

    let parsed = run.verdict.unwrap();
    assert_eq!(parsed.preference, "shadow");
    assert_eq!(parsed.confidence, Decimal::new(5, 1));
}

#[tokio::test]
async fn a_dispatch_error_gives_a_failed_call_without_cost() {
    let run = run_case(&Case::new(None)).await;

    let failure = run.verdict.unwrap_err();
    assert_eq!(failure.error, "judge call failed: judge provider exploded");
    assert_eq!(failure.cost, Decimal::ZERO);
    assert_eq!(failure.classifier_cost, Decimal::ZERO);
}

#[tokio::test]
async fn an_unqualifiable_judge_model_fails_without_reaching_the_sdk() {
    let run = run_case(
        &Case::new(Some(verdict("A", "0.9")))
            .with_router(false)
            .with_model("brand-new-judge"),
    )
    .await;

    let failure = run.verdict.unwrap_err();
    assert_eq!(
        failure.error,
        "judge call failed: judge model brand-new-judge resolves to no router deployment and no provider"
    );
    assert!(run.calls.is_empty());
}

#[tokio::test]
async fn a_public_judge_model_without_a_router_goes_through_the_sdk() {
    let run = run_case(&Case::new(Some(verdict("A", "0.9"))).with_router(false)).await;

    let call = run.calls.first().unwrap();
    assert_eq!(call.fallbacks, None);
    assert_eq!(run.verdict.unwrap().preference, "real");
}

#[tokio::test]
async fn an_unparseable_reply_names_the_parser_and_the_reply_shape_and_keeps_the_cost() {
    let run = run_case(&Case::new(Some(stamped(
        judge_response(Json::Text("not json".to_owned())),
        "0.25",
    ))))
    .await;

    let failure = run.verdict.unwrap_err();
    assert!(
        failure
            .error
            .starts_with("unparseable judge verdict: the judge reply is not JSON: ")
    );
    assert!(
        failure
            .error
            .ends_with("; finish_reason=stop, content=8 chars, model=judge-model")
    );
    assert_eq!(failure.cost, Decimal::new(25, 2));
    assert_eq!(failure.classifier_cost, Decimal::ZERO);
}

#[rstest]
#[case::missing_preference(
    r#"{"confidence": 0.5}"#,
    "unparseable judge verdict: preference: field required; "
)]
#[case::missing_confidence(
    r#"{"preference": "A"}"#,
    "unparseable judge verdict: confidence: field required; "
)]
#[case::lowercase_preference(
    r#"{"preference": "a", "confidence": 0.5}"#,
    "unparseable judge verdict: preference: input should be 'A', 'B' or 'tie'; "
)]
#[case::text_confidence(
    r#"{"preference": "A", "confidence": "high"}"#,
    "unparseable judge verdict: confidence: input should be a number; "
)]
#[case::null_confidence(
    r#"{"preference": "A", "confidence": null}"#,
    "unparseable judge verdict: confidence: input should be a number; "
)]
#[tokio::test]
async fn a_verdict_missing_or_mistyping_a_field_names_the_field(
    #[case] content: &str,
    #[case] expected: &str,
) {
    let run = run_case(&Case::new(Some(judge_response(Json::Text(
        content.to_owned(),
    )))))
    .await;

    let error = run.verdict.unwrap_err().error;
    assert!(error.starts_with(expected), "{error}");
}

#[tokio::test]
async fn a_reply_without_a_readable_message_has_the_unreadable_shape() {
    let run = run_case(&Case::new(Some(object([
        ("model", Json::Text("judge-model".to_owned())),
        ("choices", Json::List(Vec::new())),
    ]))))
    .await;

    let failure = run.verdict.unwrap_err();
    assert!(failure.error.ends_with("; unreadable judge reply"));
}

#[tokio::test]
async fn a_null_content_is_an_unparseable_verdict_that_keeps_the_cost() {
    let run = run_case(&Case::new(Some(stamped(judge_response(Json::Null), "0.5")))).await;

    let failure = run.verdict.unwrap_err();
    assert!(failure.error.starts_with("unparseable judge verdict: "));
    assert!(
        failure
            .error
            .ends_with("content=no content, model=judge-model")
    );
    assert_eq!(failure.cost, Decimal::new(5, 1));
}

#[test]
fn the_team_id_reads_only_a_non_empty_string() {
    let read = |value: Option<Json>| {
        let mut metadata = IndexMap::new();
        if let Some(value) = value {
            metadata.insert("user_api_key_team_id".to_owned(), value);
        }
        forwarded_team_id(&metadata)
    };

    assert_eq!(
        read(Some(Json::Text("team-1".to_owned()))),
        Some("team-1".to_owned())
    );
    assert_eq!(read(Some(Json::Text("".to_owned()))), None);
    assert_eq!(read(Some(Json::Null)), None);
    assert_eq!(read(Some(Json::Bool(true))), None);
    assert_eq!(read(None), None);
}

#[test]
fn the_conversation_joins_the_messages_with_a_newline() {
    let messages = vec![
        object([
            ("role", Json::Text("user".to_owned())),
            ("content", Json::Text("one".to_owned())),
        ]),
        object([
            ("role", Json::Text("assistant".to_owned())),
            ("content", Json::Text("two".to_owned())),
        ]),
    ];

    assert_eq!(conversation_text(&messages), "USER: one\nASSISTANT: two");
}

#[test]
fn a_dispatch_failure_reports_the_provider_message() {
    let failure = judge_call_failure(JudgeCallError::Dispatch {
        model: "judge".to_owned(),
        source: JudgeDispatchError {
            message: "provider down".to_owned(),
        },
    });

    assert_eq!(failure.error, "judge call failed: provider down");
    assert_eq!(failure.cost, Decimal::ZERO);
    assert_eq!(failure.classifier_cost, Decimal::ZERO);
}

#[test]
fn a_target_that_resolves_nowhere_reports_the_judge_call_error_text() {
    let failure = judge_call_failure(JudgeCallError::Nothing {
        model: "judge".to_owned(),
    });

    assert_eq!(
        failure.error,
        "judge call failed: judge model judge resolves to no router deployment and no provider"
    );
}
