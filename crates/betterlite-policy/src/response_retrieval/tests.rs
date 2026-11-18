#![cfg(test)]

use std::collections::BTreeMap;
use std::fmt::Debug;
use std::sync::{Arc, Mutex};

use tracing::field::{Field, Visit};
use tracing::subscriber::with_default;
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::layer::{Context, Layer};

use super::bucket;
use super::{
    GUARDRAIL_PIPELINES, PIPELINE_MANAGED_GUARDRAILS, PolicyPipelines, PolicyRouterView,
    ResponseRetrievalError, RetrievalAuth, RetrievalData, RetrievalDependencies, RetrievalRequest,
    UngovernedReason, UngovernedRetrieval, attach_post_call_pipelines_to_retrieval,
    model_group_for_response_id, post_call_pipelines_for_context, retrieval_context,
    submit_model_hidden_by,
};
use crate::attachment::AttachmentRegistry;
use crate::context::PolicyMatchContext;
use crate::pipeline::json_value::JsonValue;
use crate::pipeline::{GuardrailPipeline, PipelineAction, PipelineMode, PipelineStep};
use crate::registry::{PolicyRegistry, PolicySource};
use crate::values::{ModelPatterns, Policy, PolicyAttachment, PolicyCondition, PolicyGuardrails};

struct Router {
    deployments: BTreeMap<String, String>,
    aliases: Vec<(String, String)>,
}

impl Router {
    fn new(deployments: &[(&str, &str)], aliases: &[(&str, &str)]) -> Self {
        Self {
            deployments: deployments
                .iter()
                .map(|(model_id, model_name)| ((*model_id).to_owned(), (*model_name).to_owned()))
                .collect(),
            aliases: aliases
                .iter()
                .map(|(alias, target)| ((*alias).to_owned(), (*target).to_owned()))
                .collect(),
        }
    }
}

impl PolicyRouterView for Router {
    fn deployment_model_name(&self, model_id: &str) -> Option<String> {
        self.deployments.get(model_id).cloned()
    }

    fn model_group_aliases(&self) -> Vec<(String, String)> {
        self.aliases.clone()
    }
}

fn decode_with(model_id: &'static str) -> impl Fn(Option<&str>) -> Option<String> {
    move |response_id: Option<&str>| response_id.map(|_| model_id.to_owned())
}

fn decode_nothing(_response_id: Option<&str>) -> Option<String> {
    None
}

fn step(guardrail: &str) -> PipelineStep {
    PipelineStep {
        guardrail: guardrail.to_owned(),
        on_fail: PipelineAction::Block,
        on_pass: PipelineAction::Allow,
        on_error: None,
        pass_data: false,
        modify_response_message: None,
    }
}

fn pipeline(mode: PipelineMode, guardrails: &[&str]) -> GuardrailPipeline {
    GuardrailPipeline {
        mode,
        steps: guardrails.iter().copied().map(step).collect(),
    }
}

fn plain_policy() -> Policy {
    Policy {
        inherit: None,
        description: None,
        guardrails: PolicyGuardrails::default(),
        condition: None,
        pipeline: None,
    }
}

fn pipelined(mode: PipelineMode, guardrails: &[&str]) -> Policy {
    let mut value = plain_policy();
    value.pipeline = Some(pipeline(mode, guardrails));
    value
}

fn conditioned(condition_model: &str) -> Policy {
    let mut value = pipelined(PipelineMode::PostCall, &["g-cond"]);
    value.condition = Some(PolicyCondition {
        model: Some(ModelPatterns::One(condition_model.to_owned())),
    });
    value
}

fn registry(entries: Vec<(&str, Policy)>) -> PolicyRegistry {
    let mut registry = PolicyRegistry::default();
    for (name, value) in entries {
        registry.add_policy(name, value, PolicySource::Config);
    }
    registry
}

fn attachments(entries: Vec<PolicyAttachment>) -> AttachmentRegistry {
    let mut registry = AttachmentRegistry::default();
    registry.load_attachments(entries);
    registry
}

fn attached(policy_name: &str) -> PolicyAttachment {
    PolicyAttachment {
        policy: policy_name.to_owned(),
        scope: Some("*".to_owned()),
        teams: None,
        keys: None,
        models: None,
        tags: None,
        priority: None,
        default: false,
    }
}

fn text(value: &str) -> JsonValue {
    JsonValue::String(value.to_owned())
}

fn strings(values: &[&str]) -> JsonValue {
    JsonValue::Array(values.iter().map(|value| text(value)).collect())
}

fn object(entries: Vec<(&str, JsonValue)>) -> JsonValue {
    JsonValue::Object(
        entries
            .into_iter()
            .map(|(key, value)| (key.to_owned(), value))
            .collect(),
    )
}

fn data_value(entries: Vec<(&str, JsonValue)>) -> RetrievalData {
    entries
        .into_iter()
        .map(|(key, value)| (key.to_owned(), value))
        .collect()
}

fn bucket_named<'a>(data: &'a RetrievalData, name: &str) -> &'a BTreeMap<String, JsonValue> {
    match data.get(name) {
        Some(JsonValue::Object(bucket)) => bucket,
        _ => panic!("{name} bucket missing"),
    }
}

fn bucket_of(data: &RetrievalData) -> &BTreeMap<String, JsonValue> {
    bucket_named(data, bucket::METADATA)
}

fn pair_value(policy_name: &str, value: &GuardrailPipeline) -> JsonValue {
    let text = serde_json::to_string(&(policy_name, value)).expect("serialize pipeline");
    serde_json::from_str(&text).expect("decode pipeline")
}

fn stored_pipelines(bucket: &BTreeMap<String, JsonValue>) -> PolicyPipelines {
    let value = bucket.get(GUARDRAIL_PIPELINES).expect("pipelines");
    let text = serde_json::to_string(value).expect("serialize pipelines");
    serde_json::from_str(&text).expect("decode pipelines")
}

fn request() -> RetrievalRequest<'static> {
    RetrievalRequest {
        auth: RetrievalAuth::default(),
        tags: Vec::new(),
    }
}

fn dependencies<'a>(
    registry: &'a PolicyRegistry,
    attachments: &'a AttachmentRegistry,
    router: Option<&'a dyn PolicyRouterView>,
    decoder: &'a dyn Fn(Option<&str>) -> Option<String>,
) -> RetrievalDependencies<'a> {
    RetrievalDependencies {
        registry,
        attachments,
        router,
        model_id_from_response_id: decoder,
    }
}

fn run_attach(
    data: &mut RetrievalData,
    deps: &RetrievalDependencies<'_>,
) -> Result<(), ResponseRetrievalError> {
    attach_post_call_pipelines_to_retrieval(data, &request(), deps)
}

struct MessageVisitor(String);

impl Visit for MessageVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?}");
        }
    }
}

#[derive(Clone, Default)]
struct Capture(Arc<Mutex<Vec<(Level, String)>>>);

impl<S: Subscriber> Layer<S> for Capture {
    fn on_event(&self, event: &Event<'_>, _: Context<'_, S>) {
        let level = *event.metadata().level();
        if level != Level::WARN && level != Level::DEBUG {
            return;
        }
        let mut visitor = MessageVisitor(String::new());
        event.record(&mut visitor);
        if !visitor.0.starts_with("Policy engine: ") {
            return;
        }
        self.0.lock().expect("capture").push((level, visitor.0));
    }
}

impl Capture {
    fn messages(&self) -> Vec<(Level, String)> {
        self.0.lock().expect("capture").clone()
    }
}

fn captured<T>(run: impl FnOnce() -> T) -> (T, Vec<(Level, String)>) {
    let capture = Capture::default();
    let subscriber = tracing_subscriber::registry().with(capture.clone());
    let result = with_default(subscriber, run);
    (result, capture.messages())
}

#[test]
fn model_group_lookup_returns_the_deployment_model_name() {
    let router = Router::new(&[("dep-1", "gpt-4o")], &[]);
    let decoder = decode_with("dep-1");
    let (result, logs) =
        captured(|| model_group_for_response_id(Some("resp_a"), Some(&router), &decoder));
    assert_eq!(result, Ok("gpt-4o".to_owned()));
    assert!(logs.is_empty());
}

#[test]
fn model_group_lookup_without_a_router_is_ungoverned() {
    let decoder = decode_with("dep-1");
    let (result, logs) = captured(|| model_group_for_response_id(Some("resp_a"), None, &decoder));
    assert_eq!(
        result,
        Err(UngovernedRetrieval {
            reason: UngovernedReason::NoRouter
        })
    );
    assert!(logs.is_empty());
}

#[test]
fn model_group_lookup_without_a_decodable_id_is_ungoverned() {
    let router = Router::new(&[("dep-1", "gpt-4o")], &[]);
    let (result, logs) =
        captured(|| model_group_for_response_id(Some("resp_a"), Some(&router), &decode_nothing));
    assert_eq!(
        result,
        Err(UngovernedRetrieval {
            reason: UngovernedReason::ResponseIdNamesNoDeployment
        })
    );
    assert!(logs.is_empty());
}

#[test]
fn model_group_lookup_for_a_missing_deployment_is_ungoverned() {
    let router = Router::new(&[("dep-1", "gpt-4o")], &[]);
    let decoder = decode_with("gone-id");
    let (result, logs) =
        captured(|| model_group_for_response_id(Some("resp_a"), Some(&router), &decoder));
    assert_eq!(
        result,
        Err(UngovernedRetrieval {
            reason: UngovernedReason::DeploymentNoLongerInRouter
        })
    );
    assert!(logs.is_empty());
}

#[test]
fn wildcard_deployment_warns_that_the_submission_name_is_hidden() {
    let router = Router::new(&[("dep-1", "gpt-4o-*")], &[]);
    let decoder = decode_with("dep-1");
    let (result, logs) =
        captured(|| model_group_for_response_id(Some("resp_a"), Some(&router), &decoder));
    assert_eq!(result, Ok("gpt-4o-*".to_owned()));
    assert_eq!(
        logs,
        vec![(
            Level::WARN,
            "Policy engine: background response resp_a re-matches policies on retrieval as model \
             group gpt-4o-* (a wildcard deployment), so a policy attached to the model name it was \
             submitted as does not run on it"
                .to_owned()
        )]
    );
}

#[test]
fn alias_target_warns_with_every_alias() {
    let router = Router::new(
        &[("dep-1", "gpt-4o")],
        &[
            ("alias-one", "gpt-4o"),
            ("alias-two", "gpt-4o"),
            ("unrelated", "other"),
        ],
    );
    let decoder = decode_with("dep-1");
    let (result, logs) =
        captured(|| model_group_for_response_id(Some("resp_a"), Some(&router), &decoder));
    assert_eq!(result, Ok("gpt-4o".to_owned()));
    assert_eq!(
        logs,
        vec![(
            Level::WARN,
            "Policy engine: background response resp_a re-matches policies on retrieval as model \
             group gpt-4o (the target of model_group_alias alias-one, alias-two), so a policy \
             attached to the model name it was submitted as does not run on it"
                .to_owned()
        )]
    );
}

#[test]
fn an_alias_targeting_another_group_does_not_warn() {
    let router = Router::new(&[("dep-1", "gpt-4o")], &[("alias-three", "other")]);
    let decoder = decode_with("dep-1");
    let (result, logs) =
        captured(|| model_group_for_response_id(Some("resp_a"), Some(&router), &decoder));
    assert_eq!(result, Ok("gpt-4o".to_owned()));
    assert!(logs.is_empty());
}

#[test]
fn wildcard_names_win_over_aliases_when_the_name_is_hidden() {
    let aliases = vec![("alias-one".to_owned(), "gpt-*".to_owned())];
    assert_eq!(
        submit_model_hidden_by("gpt-*", &aliases),
        Some("a wildcard deployment".to_owned())
    );
}

#[test]
fn submit_model_hidden_by_names_matching_aliases_in_order() {
    let aliases = vec![
        ("alias-one".to_owned(), "gpt-4o".to_owned()),
        ("alias-two".to_owned(), "claude".to_owned()),
        ("alias-three".to_owned(), "gpt-4o".to_owned()),
    ];
    assert_eq!(
        submit_model_hidden_by("gpt-4o", &aliases),
        Some("the target of model_group_alias alias-one, alias-three".to_owned())
    );
    assert_eq!(submit_model_hidden_by("gpt-4o", &[]), None);
    assert_eq!(submit_model_hidden_by("gpt-4o", &aliases[1..2]), None);
}

#[test]
fn retrieval_context_carries_aliases_model_and_tags() {
    let auth = RetrievalAuth {
        team_alias: Some("team-a"),
        key_alias: Some("key-1"),
    };
    let context = retrieval_context(&auth, "gpt-4o", vec!["prod-eu".to_owned()]);
    assert_eq!(
        context,
        PolicyMatchContext {
            team_alias: Some("team-a".to_owned()),
            key_alias: Some("key-1".to_owned()),
            model: Some("gpt-4o".to_owned()),
            tags: Some(vec!["prod-eu".to_owned()]),
        }
    );
}

#[test]
fn retrieval_context_leaves_unset_aliases_and_empty_tags_empty() {
    let context = retrieval_context(&RetrievalAuth::default(), "gpt-4o", Vec::new());
    assert_eq!(
        context,
        PolicyMatchContext {
            team_alias: None,
            key_alias: None,
            model: Some("gpt-4o".to_owned()),
            tags: None,
        }
    );
}

#[test]
fn post_call_pipelines_are_filtered_and_sources_cover_every_match() {
    let registry = registry(vec![
        ("post", pipelined(PipelineMode::PostCall, &["g-post"])),
        ("pre", pipelined(PipelineMode::PreCall, &["g-pre"])),
        ("guards", plain_policy()),
    ]);
    let attachments = attachments(vec![attached("post"), attached("pre"), attached("guards")]);
    let context = retrieval_context(&RetrievalAuth::default(), "gpt-4o", Vec::new());
    let found = post_call_pipelines_for_context(&context, &registry, &attachments);
    assert_eq!(
        found.pipelines,
        vec![(
            "post".to_owned(),
            pipeline(PipelineMode::PostCall, &["g-post"])
        )]
    );
    let sources: BTreeMap<String, String> = [
        ("post", "scope:*"),
        ("pre", "scope:*"),
        ("guards", "scope:*"),
    ]
    .into_iter()
    .map(|(name, reason)| (name.to_owned(), reason.to_owned()))
    .collect();
    assert_eq!(found.sources, sources);
}

#[test]
fn post_call_pipelines_without_matches_are_empty() {
    let registry = registry(vec![(
        "post",
        pipelined(PipelineMode::PostCall, &["g-post"]),
    )]);
    let attachments = attachments(Vec::new());
    let context = retrieval_context(&RetrievalAuth::default(), "gpt-4o", Vec::new());
    let found = post_call_pipelines_for_context(&context, &registry, &attachments);
    assert!(found.pipelines.is_empty());
    assert!(found.sources.is_empty());
}

#[test]
fn attaches_post_call_pipelines_with_provenance() {
    let registry = registry(vec![
        ("p1", pipelined(PipelineMode::PostCall, &["g1", "g2"])),
        ("p2", pipelined(PipelineMode::PostCall, &["g2", "g3"])),
    ]);
    let attachments = attachments(vec![attached("p1"), attached("p2")]);
    let router = Router::new(&[("dep-1", "gpt-4o")], &[]);
    let decoder = decode_with("dep-1");
    let mut data = data_value(vec![
        ("response_id", text("resp_a")),
        ("model", text("gpt-4o")),
    ]);
    let deps = dependencies(&registry, &attachments, Some(&router), &decoder);
    let (result, logs) = captured(|| run_attach(&mut data, &deps));
    assert!(result.is_ok());
    assert_eq!(data.get("model"), Some(&text("gpt-4o")));
    let bucket = bucket_of(&data);
    assert_eq!(
        stored_pipelines(bucket),
        vec![
            (
                "p1".to_owned(),
                pipeline(PipelineMode::PostCall, &["g1", "g2"])
            ),
            (
                "p2".to_owned(),
                pipeline(PipelineMode::PostCall, &["g2", "g3"])
            ),
        ]
    );
    assert_eq!(
        bucket.get(PIPELINE_MANAGED_GUARDRAILS),
        Some(&strings(&["g1", "g2", "g3"]))
    );
    assert_eq!(
        bucket.get(bucket::APPLIED_POLICIES),
        Some(&strings(&["p1", "p2"]))
    );
    assert_eq!(
        bucket.get(bucket::APPLIED_GUARDRAILS),
        Some(&strings(&["g1", "g2", "g3"]))
    );
    assert_eq!(
        bucket.get(bucket::POLICY_SOURCES),
        Some(&object(vec![
            ("p1", text("scope:*")),
            ("p2", text("scope:*")),
        ]))
    );
    assert_eq!(
        logs,
        vec![(
            Level::DEBUG,
            "Policy engine: attached post_call pipelines to the retrieval of background response \
             resp_a (model group gpt-4o): p1, p2"
                .to_owned()
        )]
    );
}

#[test]
fn writes_to_the_litellm_metadata_bucket_when_present() {
    let registry = registry(vec![("p1", pipelined(PipelineMode::PostCall, &["g1"]))]);
    let attachments = attachments(vec![attached("p1")]);
    let router = Router::new(&[("dep-1", "gpt-4o")], &[]);
    let decoder = decode_with("dep-1");
    let mut data = data_value(vec![
        ("response_id", text("resp_a")),
        ("metadata", object(vec![("caller", text("kept"))])),
        ("litellm_metadata", object(vec![("caller", text("kept"))])),
    ]);
    let deps = dependencies(&registry, &attachments, Some(&router), &decoder);
    assert!(run_attach(&mut data, &deps).is_ok());
    let preferred = bucket_named(&data, bucket::LITELLM_METADATA);
    assert!(preferred.contains_key(GUARDRAIL_PIPELINES));
    assert_eq!(
        preferred.get(bucket::APPLIED_POLICIES),
        Some(&strings(&["p1"]))
    );
    assert_eq!(
        bucket_of(&data),
        &BTreeMap::from([("caller".to_owned(), text("kept"))])
    );
}

#[test]
fn null_litellm_metadata_uses_the_public_metadata_bucket() {
    let registry = registry(vec![("p1", pipelined(PipelineMode::PostCall, &["g1"]))]);
    let attachments = attachments(vec![attached("p1")]);
    let router = Router::new(&[("dep-1", "gpt-4o")], &[]);
    let decoder = decode_with("dep-1");
    let mut data = data_value(vec![
        ("response_id", text("resp_a")),
        ("metadata", object(vec![("caller", text("kept"))])),
        ("litellm_metadata", JsonValue::Null),
    ]);
    let deps = dependencies(&registry, &attachments, Some(&router), &decoder);

    assert!(run_attach(&mut data, &deps).is_ok());
    assert!(bucket_of(&data).contains_key(GUARDRAIL_PIPELINES));
    assert_eq!(data.get("litellm_metadata"), Some(&JsonValue::Null));
}

#[test]
fn creates_the_metadata_bucket_when_no_policies_match() {
    let registry = registry(vec![("p1", pipelined(PipelineMode::PostCall, &["g1"]))]);
    let attachments = attachments(Vec::new());
    let router = Router::new(&[("dep-1", "gpt-4o")], &[]);
    let decoder = decode_with("dep-1");
    let mut data = data_value(vec![("response_id", text("resp_a"))]);
    let deps = dependencies(&registry, &attachments, Some(&router), &decoder);
    let (result, logs) = captured(|| run_attach(&mut data, &deps));
    assert!(result.is_ok());
    assert_eq!(bucket_of(&data), &BTreeMap::new());
    assert!(logs.is_empty());
}

#[test]
fn uninitialized_registry_leaves_the_data_untouched() {
    let registry = PolicyRegistry::default();
    let attachments = attachments(vec![attached("p1")]);
    let router = Router::new(&[("dep-1", "gpt-4o")], &[]);
    let decoder = decode_with("dep-1");
    let mut data = data_value(vec![("response_id", text("resp_a"))]);
    let before = data.clone();
    let deps = dependencies(&registry, &attachments, Some(&router), &decoder);
    let (result, logs) = captured(|| run_attach(&mut data, &deps));
    assert!(result.is_ok());
    assert_eq!(data, before);
    assert!(logs.is_empty());
}

#[test]
fn ungoverned_retrieval_warns_and_leaves_the_data_untouched() {
    let registry = registry(vec![("p1", pipelined(PipelineMode::PostCall, &["g1"]))]);
    let attachments = attachments(vec![attached("p1")]);
    let router = Router::new(&[("dep-1", "gpt-4o")], &[]);
    let decoder = decode_with("dep-1");

    let mut without_router = data_value(vec![("response_id", text("resp_a"))]);
    let before = without_router.clone();
    let deps = dependencies(&registry, &attachments, None, &decoder);
    let (result, logs) = captured(|| run_attach(&mut without_router, &deps));
    assert!(result.is_ok());
    assert_eq!(without_router, before);
    assert_eq!(
        logs,
        vec![(
            Level::WARN,
            "Policy engine: background response resp_a is retrieved without its post_call policy \
             pipelines (no router)"
                .to_owned()
        )]
    );

    let mut without_id = data_value(vec![("model", text("gpt-4o"))]);
    let before = without_id.clone();
    let deps = dependencies(&registry, &attachments, Some(&router), &decoder);
    let (result, logs) = captured(|| run_attach(&mut without_id, &deps));
    assert!(result.is_ok());
    assert_eq!(without_id, before);
    assert_eq!(
        logs,
        vec![(
            Level::WARN,
            "Policy engine: background response None is retrieved without its post_call policy \
             pipelines (response id names no deployment)"
                .to_owned()
        )]
    );

    let gone = decode_with("gone-id");
    let mut missing_deployment = data_value(vec![("response_id", text("resp_a"))]);
    let before = missing_deployment.clone();
    let deps = dependencies(&registry, &attachments, Some(&router), &gone);
    let (result, logs) = captured(|| run_attach(&mut missing_deployment, &deps));
    assert!(result.is_ok());
    assert_eq!(missing_deployment, before);
    assert_eq!(
        logs,
        vec![(
            Level::WARN,
            "Policy engine: background response resp_a is retrieved without its post_call policy \
             pipelines (deployment no longer in the router)"
                .to_owned()
        )]
    );
}

#[test]
fn already_attached_policies_are_not_attached_again() {
    let existing = pipeline(PipelineMode::PostCall, &["existing-guard"]);
    let metadata = object(vec![
        (
            GUARDRAIL_PIPELINES,
            JsonValue::Array(vec![pair_value("existing-policy", &existing)]),
        ),
        (bucket::APPLIED_POLICIES, strings(&["existing-policy"])),
        (bucket::APPLIED_GUARDRAILS, strings(&["existing-guard"])),
        (
            bucket::POLICY_SOURCES,
            object(vec![("existing-policy", text("scope:*"))]),
        ),
    ]);
    let mut data = data_value(vec![
        ("response_id", text("resp_a")),
        ("metadata", metadata),
    ]);
    let registry = registry(vec![
        (
            "existing-policy",
            pipelined(PipelineMode::PostCall, &["existing-guard"]),
        ),
        (
            "new-policy",
            pipelined(PipelineMode::PostCall, &["new-guard"]),
        ),
    ]);
    let attachments = attachments(vec![attached("existing-policy"), attached("new-policy")]);
    let router = Router::new(&[("dep-1", "gpt-4o")], &[]);
    let decoder = decode_with("dep-1");
    let deps = dependencies(&registry, &attachments, Some(&router), &decoder);
    let (result, logs) = captured(|| run_attach(&mut data, &deps));
    assert!(result.is_ok());
    let bucket = bucket_of(&data);
    assert_eq!(
        stored_pipelines(bucket),
        vec![
            ("existing-policy".to_owned(), existing),
            (
                "new-policy".to_owned(),
                pipeline(PipelineMode::PostCall, &["new-guard"])
            ),
        ]
    );
    assert_eq!(
        bucket.get(PIPELINE_MANAGED_GUARDRAILS),
        Some(&strings(&["existing-guard", "new-guard"]))
    );
    assert_eq!(
        bucket.get(bucket::APPLIED_POLICIES),
        Some(&strings(&["existing-policy", "new-policy"]))
    );
    assert_eq!(
        bucket.get(bucket::APPLIED_GUARDRAILS),
        Some(&strings(&["existing-guard", "new-guard"]))
    );
    assert_eq!(
        bucket.get(bucket::POLICY_SOURCES),
        Some(&object(vec![
            ("existing-policy", text("scope:*")),
            ("new-policy", text("scope:*")),
        ]))
    );
    assert_eq!(
        logs,
        vec![(
            Level::DEBUG,
            "Policy engine: attached post_call pipelines to the retrieval of background response \
             resp_a (model group gpt-4o): new-policy"
                .to_owned()
        )]
    );
}

#[test]
fn a_fully_attached_bucket_is_left_alone() {
    let existing = pipeline(PipelineMode::PostCall, &["existing-guard"]);
    let stored = JsonValue::Array(vec![pair_value("existing-policy", &existing)]);
    let metadata = object(vec![(GUARDRAIL_PIPELINES, stored.clone())]);
    let mut data = data_value(vec![
        ("response_id", text("resp_a")),
        ("metadata", metadata),
    ]);
    let registry = registry(vec![(
        "existing-policy",
        pipelined(PipelineMode::PostCall, &["existing-guard"]),
    )]);
    let attachments = attachments(vec![attached("existing-policy")]);
    let router = Router::new(&[("dep-1", "gpt-4o")], &[]);
    let decoder = decode_with("dep-1");
    let deps = dependencies(&registry, &attachments, Some(&router), &decoder);
    let (result, logs) = captured(|| run_attach(&mut data, &deps));
    assert!(result.is_ok());
    let bucket = bucket_of(&data);
    assert_eq!(bucket.get(GUARDRAIL_PIPELINES), Some(&stored));
    assert!(bucket.get(PIPELINE_MANAGED_GUARDRAILS).is_none());
    assert!(bucket.get(bucket::APPLIED_POLICIES).is_none());
    assert!(logs.is_empty());
}

#[test]
fn condition_filtered_policies_are_not_attached() {
    let registry = registry(vec![
        ("matching", conditioned("gpt-4.*")),
        ("other-model", conditioned("claude")),
    ]);
    let attachments = attachments(vec![attached("matching"), attached("other-model")]);
    let router = Router::new(&[("dep-1", "gpt-4o")], &[]);
    let decoder = decode_with("dep-1");
    let mut data = data_value(vec![("response_id", text("resp_a"))]);
    let deps = dependencies(&registry, &attachments, Some(&router), &decoder);
    assert!(run_attach(&mut data, &deps).is_ok());
    let bucket = bucket_of(&data);
    assert_eq!(
        stored_pipelines(bucket),
        vec![(
            "matching".to_owned(),
            pipeline(PipelineMode::PostCall, &["g-cond"])
        )]
    );
    assert_eq!(
        bucket.get(bucket::POLICY_SOURCES),
        Some(&object(vec![("matching", text("scope:*"))]))
    );
}

#[test]
fn non_object_metadata_is_replaced_by_a_bucket() {
    let registry = registry(vec![("p1", pipelined(PipelineMode::PostCall, &["g1"]))]);
    let attachments = attachments(vec![attached("p1")]);
    let router = Router::new(&[("dep-1", "gpt-4o")], &[]);
    let decoder = decode_with("dep-1");
    let mut data = data_value(vec![
        ("response_id", text("resp_a")),
        ("metadata", text("junk")),
    ]);
    let deps = dependencies(&registry, &attachments, Some(&router), &decoder);
    assert!(run_attach(&mut data, &deps).is_ok());
    let bucket = bucket_of(&data);
    assert!(bucket.contains_key(GUARDRAIL_PIPELINES));
    assert_eq!(
        bucket.get(bucket::APPLIED_POLICIES),
        Some(&strings(&["p1"]))
    );
}

#[test]
fn a_non_list_guardrail_pipelines_value_is_an_error() {
    let registry = registry(vec![("p1", pipelined(PipelineMode::PostCall, &["g1"]))]);
    let attachments = attachments(vec![attached("p1")]);
    let router = Router::new(&[("dep-1", "gpt-4o")], &[]);
    let decoder = decode_with("dep-1");
    let mut data = data_value(vec![
        ("response_id", text("resp_a")),
        (
            "metadata",
            object(vec![(GUARDRAIL_PIPELINES, text("junk"))]),
        ),
    ]);
    let deps = dependencies(&registry, &attachments, Some(&router), &decoder);
    let result = run_attach(&mut data, &deps);
    assert!(matches!(
        result,
        Err(ResponseRetrievalError::NotTyped { field, .. }) if field == GUARDRAIL_PIPELINES
    ));
}

#[test]
fn a_null_guardrail_pipelines_value_reads_as_empty() {
    let registry = registry(vec![("p1", pipelined(PipelineMode::PostCall, &["g1"]))]);
    let attachments = attachments(vec![attached("p1")]);
    let router = Router::new(&[("dep-1", "gpt-4o")], &[]);
    let decoder = decode_with("dep-1");
    let mut data = data_value(vec![
        ("response_id", text("resp_a")),
        (
            "metadata",
            object(vec![(GUARDRAIL_PIPELINES, JsonValue::Null)]),
        ),
    ]);
    let deps = dependencies(&registry, &attachments, Some(&router), &decoder);
    assert!(run_attach(&mut data, &deps).is_ok());
    assert_eq!(
        stored_pipelines(bucket_of(&data)),
        vec![("p1".to_owned(), pipeline(PipelineMode::PostCall, &["g1"]))]
    );
}

#[test]
fn a_malformed_stored_pipeline_is_an_error() {
    let broken = JsonValue::Array(vec![JsonValue::Array(vec![
        text("p1"),
        object(vec![
            ("mode", text("pre_call")),
            ("steps", JsonValue::Array(Vec::new())),
        ]),
    ])]);
    let registry = registry(vec![("p1", pipelined(PipelineMode::PostCall, &["g1"]))]);
    let attachments = attachments(vec![attached("p1")]);
    let router = Router::new(&[("dep-1", "gpt-4o")], &[]);
    let decoder = decode_with("dep-1");
    let mut data = data_value(vec![
        ("response_id", text("resp_a")),
        ("metadata", object(vec![(GUARDRAIL_PIPELINES, broken)])),
    ]);
    let deps = dependencies(&registry, &attachments, Some(&router), &decoder);
    let result = run_attach(&mut data, &deps);
    assert!(matches!(result, Err(ResponseRetrievalError::Pipelines(_))));
}

#[test]
fn a_non_list_applied_policies_header_is_an_error() {
    let registry = registry(vec![("p1", pipelined(PipelineMode::PostCall, &["g1"]))]);
    let attachments = attachments(vec![attached("p1")]);
    let router = Router::new(&[("dep-1", "gpt-4o")], &[]);
    let decoder = decode_with("dep-1");
    let mut data = data_value(vec![
        ("response_id", text("resp_a")),
        (
            "metadata",
            object(vec![(
                bucket::APPLIED_POLICIES,
                JsonValue::Number(serde_json::Number::from(5_u8)),
            )]),
        ),
    ]);
    let deps = dependencies(&registry, &attachments, Some(&router), &decoder);
    let result = run_attach(&mut data, &deps);
    assert!(matches!(
        result,
        Err(ResponseRetrievalError::NotTyped { field, .. }) if field == bucket::APPLIED_POLICIES
    ));
}

#[test]
fn existing_headers_and_sources_are_preserved() {
    let metadata = object(vec![
        (bucket::APPLIED_POLICIES, strings(&["old-policy"])),
        (bucket::APPLIED_GUARDRAILS, strings(&["g-old"])),
        (
            bucket::POLICY_SOURCES,
            object(vec![("old-policy", text("team:team-a"))]),
        ),
    ]);
    let mut data = data_value(vec![
        ("response_id", text("resp_a")),
        ("metadata", metadata),
    ]);
    let registry = registry(vec![("p1", pipelined(PipelineMode::PostCall, &["g1"]))]);
    let attachments = attachments(vec![attached("p1")]);
    let router = Router::new(&[("dep-1", "gpt-4o")], &[]);
    let decoder = decode_with("dep-1");
    let deps = dependencies(&registry, &attachments, Some(&router), &decoder);
    assert!(run_attach(&mut data, &deps).is_ok());
    let bucket = bucket_of(&data);
    assert_eq!(
        bucket.get(bucket::APPLIED_POLICIES),
        Some(&strings(&["old-policy", "p1"]))
    );
    assert_eq!(
        bucket.get(bucket::APPLIED_GUARDRAILS),
        Some(&strings(&["g-old", "g1"]))
    );
    assert_eq!(
        bucket.get(bucket::POLICY_SOURCES),
        Some(&object(vec![
            ("old-policy", text("team:team-a")),
            ("p1", text("scope:*")),
        ]))
    );
}

#[test]
fn a_non_object_policy_sources_value_is_replaced() {
    let mut data = data_value(vec![
        ("response_id", text("resp_a")),
        (
            "metadata",
            object(vec![(bucket::POLICY_SOURCES, text("junk"))]),
        ),
    ]);
    let registry = registry(vec![("p1", pipelined(PipelineMode::PostCall, &["g1"]))]);
    let attachments = attachments(vec![attached("p1")]);
    let router = Router::new(&[("dep-1", "gpt-4o")], &[]);
    let decoder = decode_with("dep-1");
    let deps = dependencies(&registry, &attachments, Some(&router), &decoder);
    assert!(run_attach(&mut data, &deps).is_ok());
    assert_eq!(
        bucket_of(&data).get(bucket::POLICY_SOURCES),
        Some(&object(vec![("p1", text("scope:*"))]))
    );
}

#[test]
fn null_applied_headers_are_treated_as_empty() {
    let mut data = data_value(vec![
        ("response_id", text("resp_a")),
        (
            "metadata",
            object(vec![
                (bucket::APPLIED_POLICIES, JsonValue::Null),
                (bucket::APPLIED_GUARDRAILS, JsonValue::Null),
            ]),
        ),
    ]);
    let registry = registry(vec![("p1", pipelined(PipelineMode::PostCall, &["g1"]))]);
    let attachments = attachments(vec![attached("p1")]);
    let router = Router::new(&[("dep-1", "gpt-4o")], &[]);
    let decoder = decode_with("dep-1");
    let deps = dependencies(&registry, &attachments, Some(&router), &decoder);
    assert!(run_attach(&mut data, &deps).is_ok());
    let bucket = bucket_of(&data);
    assert_eq!(
        bucket.get(bucket::APPLIED_POLICIES),
        Some(&strings(&["p1"]))
    );
    assert_eq!(
        bucket.get(bucket::APPLIED_GUARDRAILS),
        Some(&strings(&["g1"]))
    );
}
