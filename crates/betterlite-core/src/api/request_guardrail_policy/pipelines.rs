use betterlite_models::decisions::Json;
use betterlite_policy::pipeline::{GuardrailPipeline, PipelineAction, PipelineMode};
use betterlite_policy::resolver::pipelines::pipeline_managed_guardrails;
use indexmap::IndexMap;

const GUARDRAIL_PIPELINES: &str = "_guardrail_pipelines";
const MANAGED_GUARDRAILS: &str = "_pipeline_managed_guardrails";

pub(super) fn write_pipelines(
    bucket: &mut IndexMap<String, Json>,
    pipelines: &[(String, GuardrailPipeline)],
) {
    if pipelines.is_empty() {
        return;
    }
    let managed = pipeline_managed_guardrails(pipelines);
    tracing::debug!(
        "Policy engine: resolved {} pipeline(s), managed guardrails: {:?}",
        pipelines.len(),
        managed
    );
    bucket.insert(GUARDRAIL_PIPELINES.to_owned(), pipelines_json(pipelines));
    bucket.insert(
        MANAGED_GUARDRAILS.to_owned(),
        Json::List(managed.into_iter().map(Json::Text).collect()),
    );
}

fn pipelines_json(pipelines: &[(String, GuardrailPipeline)]) -> Json {
    let mut entries = Vec::with_capacity(pipelines.len());
    for (policy_name, pipeline) in pipelines {
        entries.push(Json::List(vec![
            Json::Text(policy_name.clone()),
            pipeline_json(pipeline),
        ]));
    }
    Json::List(entries)
}

fn pipeline_json(pipeline: &GuardrailPipeline) -> Json {
    let mut steps = Vec::with_capacity(pipeline.steps.len());
    for step in &pipeline.steps {
        let mut fields = IndexMap::new();
        fields.insert("guardrail".to_owned(), Json::Text(step.guardrail.clone()));
        fields.insert(
            "on_fail".to_owned(),
            Json::Text(pipeline_action_name(step.on_fail).to_owned()),
        );
        fields.insert(
            "on_pass".to_owned(),
            Json::Text(pipeline_action_name(step.on_pass).to_owned()),
        );
        fields.insert(
            "on_error".to_owned(),
            step.on_error.map_or(Json::Null, |action| {
                Json::Text(pipeline_action_name(action).to_owned())
            }),
        );
        fields.insert("pass_data".to_owned(), Json::Bool(step.pass_data));
        fields.insert(
            "modify_response_message".to_owned(),
            step.modify_response_message
                .as_ref()
                .map_or(Json::Null, |message| Json::Text(message.clone())),
        );
        steps.push(Json::Object(fields));
    }
    let mut fields = IndexMap::new();
    fields.insert(
        "mode".to_owned(),
        Json::Text(pipeline_mode_name(pipeline.mode).to_owned()),
    );
    fields.insert("steps".to_owned(), Json::List(steps));
    Json::Object(fields)
}

const fn pipeline_mode_name(mode: PipelineMode) -> &'static str {
    match mode {
        PipelineMode::PreCall => "pre_call",
        PipelineMode::PostCall => "post_call",
    }
}

const fn pipeline_action_name(action: PipelineAction) -> &'static str {
    match action {
        PipelineAction::Allow => "allow",
        PipelineAction::Block => "block",
        PipelineAction::Next => "next",
        PipelineAction::ModifyResponse => "modify_response",
    }
}
