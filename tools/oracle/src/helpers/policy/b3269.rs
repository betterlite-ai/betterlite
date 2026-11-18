use std::collections::{BTreeMap, BTreeSet};

use betterlite_core::api::response_id_codecs::response::get_model_id_from_response_id;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_policy::attachment::AttachmentRegistry;
use betterlite_policy::pipeline::{GuardrailPipeline, PipelineAction, PipelineMode, PipelineStep};
use betterlite_policy::registry::{PolicyRegistry, PolicySource};
use betterlite_policy::resolver::pipelines::pipeline_managed_guardrails;
use betterlite_policy::response_retrieval::{
    PolicyRouterView, RetrievalAuth, RetrievalData, RetrievalDependencies, RetrievalRequest,
    attach_retrieval_pipelines, model_group_for_response_id, post_call_pipelines,
    retrieval_context as context_for_retrieval, submit_model_hidden_by,
};
use betterlite_policy::values::{Policy, PolicyAttachment, PolicyGuardrails};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

pub(crate) struct Op;

impl Helper for Op {
    type Input = Value;

    fn run(input: &Self::Input) -> HelperResult {
        let function = input
            .get("function")
            .and_then(Value::as_str)
            .ok_or_else(|| invalid_input(String::from("function must be a string")))?;
        match function {
            "model_group_lookup" => model_group_lookup(input),
            "submit_model_hidden_by" => hidden_by_action(input),
            "retrieval_context" => context_building(input),
            "pipeline_filtering" | "deduplication" => pipeline_filtering(input),
            "registry_not_initialized" => registry_not_initialized(),
            _ => Err(invalid_input(format!(
                "unknown b3269 helper function {function}"
            ))),
        }
    }
}

struct Router {
    deployments: BTreeMap<String, String>,
    aliases: Vec<(String, String)>,
}

impl PolicyRouterView for Router {
    fn deployment_model_name(&self, model_id: &str) -> Option<String> {
        self.deployments.get(model_id).cloned()
    }

    fn model_group_aliases(&self) -> Vec<(String, String)> {
        self.aliases.clone()
    }
}

fn model_group_lookup(input: &Value) -> HelperResult {
    let deployments: BTreeMap<String, String> =
        decoded_or_default(input.get("deployments"), "deployments")?;
    let aliases: Vec<(String, String)> = decoded_or_default(input.get("aliases"), "aliases")?;
    let router = input
        .get("router")
        .and_then(Value::as_bool)
        .unwrap_or(false)
        .then_some(Router {
            deployments,
            aliases: aliases.clone(),
        });
    let view: Option<&dyn PolicyRouterView> = match &router {
        Some(instance) => Some(instance),
        None => None,
    };
    let response_id = input.get("response_id").and_then(Value::as_str);
    let decoder: &dyn Fn(Option<&str>) -> Option<String> = &get_model_id_from_response_id;
    match model_group_for_response_id(response_id, view, decoder) {
        Ok(model_group) => Ok(json!({
            "model_group": model_group,
            "reason": null,
            "hidden_by": submit_model_hidden_by(&model_group, &aliases),
        })),
        Err(ungoverned) => Ok(json!({
            "model_group": null,
            "reason": ungoverned.reason.message(),
            "hidden_by": null,
        })),
    }
}

fn hidden_by_action(input: &Value) -> HelperResult {
    let model_group = string_field(input, "model_group")?;
    let aliases: Vec<(String, String)> = decoded_or_default(input.get("aliases"), "aliases")?;
    Ok(json!({
        "hidden_by": submit_model_hidden_by(model_group, &aliases),
    }))
}

fn context_building(input: &Value) -> HelperResult {
    let model_group = string_field(input, "model_group")?;
    let team_alias = auth_field(input, "team_alias")?;
    let key_alias = auth_field(input, "key_alias")?;
    let tags: Vec<String> = decoded_or_default(input.get("tags"), "tags")?;
    let context = context_for_retrieval(
        &RetrievalAuth {
            team_alias,
            key_alias,
        },
        model_group,
        tags,
    );
    serde_json::to_value(context).map_err(|error| invalid_input(error.to_string()))
}

#[derive(Deserialize)]
struct PipelineInput {
    policy_name: String,
    mode: PipelineMode,
    guardrails: Vec<String>,
}

fn pipeline_filtering(input: &Value) -> HelperResult {
    let pipelines = decoded_pipelines(input.get("pipelines"), "pipelines")?;
    let already_attached = decoded_pipelines(input.get("already_attached"), "already_attached")?;
    let (registry, attachments) = registry_for(&pipelines);
    let context = context_for_retrieval(&RetrievalAuth::default(), "model", Vec::new());
    let found = post_call_pipelines(&context, &registry, &attachments);
    let attached_names: BTreeSet<String> = already_attached
        .iter()
        .map(|(policy_name, _)| policy_name.clone())
        .collect();
    let mut merged = already_attached;
    let mut added = Vec::new();
    for (policy_name, pipeline) in &found.pipelines {
        if attached_names.contains(policy_name) {
            continue;
        }
        added.push(pipeline_value(policy_name, pipeline));
        merged.push((policy_name.clone(), pipeline.clone()));
    }
    let managed: Vec<String> = pipeline_managed_guardrails(&merged).into_iter().collect();
    Ok(json!({"pipelines": added, "managed_guardrails": managed}))
}

fn registry_not_initialized() -> HelperResult {
    let registry = PolicyRegistry::default();
    let attachments = AttachmentRegistry::default();
    let decoder: &dyn Fn(Option<&str>) -> Option<String> = &get_model_id_from_response_id;
    let deps = RetrievalDependencies {
        registry: &registry,
        attachments: &attachments,
        router: None,
        model_id_from_response_id: decoder,
    };
    let mut data = RetrievalData::new();
    attach_retrieval_pipelines(
        &mut data,
        &RetrievalRequest {
            auth: RetrievalAuth::default(),
            tags: Vec::new(),
        },
        &deps,
    )
    .map_err(|error| invalid_input(error.to_string()))?;
    Ok(json!({
        "attached": data.contains_key("metadata") || data.contains_key("litellm_metadata"),
    }))
}

fn decoded_pipelines(
    value: Option<&Value>,
    field: &str,
) -> Result<Vec<(String, GuardrailPipeline)>, Box<HelperError>> {
    let entries: Vec<PipelineInput> = decoded_or_default(value, field)?;
    Ok(entries
        .into_iter()
        .map(|entry| {
            let steps = entry
                .guardrails
                .iter()
                .map(|guardrail| step(guardrail.as_str()))
                .collect();
            (
                entry.policy_name,
                GuardrailPipeline {
                    mode: entry.mode,
                    steps,
                },
            )
        })
        .collect())
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

fn registry_for(pipelines: &[(String, GuardrailPipeline)]) -> (PolicyRegistry, AttachmentRegistry) {
    let mut registry = PolicyRegistry::default();
    let mut attachments = AttachmentRegistry::default();
    for (policy_name, pipeline) in pipelines {
        registry.add_policy(
            policy_name,
            Policy {
                inherit: None,
                description: None,
                guardrails: PolicyGuardrails::default(),
                condition: None,
                pipeline: Some(pipeline.clone()),
            },
            PolicySource::Config,
        );
        attachments.add_attachment(PolicyAttachment {
            policy: policy_name.clone(),
            scope: Some("*".to_owned()),
            teams: None,
            keys: None,
            models: None,
            tags: None,
            priority: None,
            default: false,
        });
    }
    (registry, attachments)
}

fn pipeline_value(policy_name: &str, pipeline: &GuardrailPipeline) -> Value {
    let guardrails: Vec<String> = pipeline
        .steps
        .iter()
        .map(|pipeline_step| pipeline_step.guardrail.clone())
        .collect();
    json!({
        "policy_name": policy_name,
        "mode": pipeline.mode,
        "guardrails": guardrails,
    })
}

fn decoded_or_default<T>(value: Option<&Value>, field: &str) -> Result<T, Box<HelperError>>
where
    T: Default + DeserializeOwned,
{
    let Some(value) = value else {
        return Ok(T::default());
    };
    serde_json::from_value(value.clone())
        .map_err(|error| invalid_input(format!("{field}: {error}")))
}

fn string_field<'input>(
    input: &'input Value,
    field: &str,
) -> Result<&'input str, Box<HelperError>> {
    let Some(value) = input.get(field).and_then(Value::as_str) else {
        return Err(invalid_input(format!("{field} must be a string")));
    };
    Ok(value)
}

fn auth_field<'input>(
    input: &'input Value,
    field: &str,
) -> Result<Option<&'input str>, Box<HelperError>> {
    let Some(auth) = input.get("auth") else {
        return Ok(None);
    };
    match auth.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) => Ok(Some(value.as_str())),
        Some(_) => Err(invalid_input(format!(
            "auth.{field} must be a string or null"
        ))),
    }
}

fn invalid_input(message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: "InvalidInput".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}
