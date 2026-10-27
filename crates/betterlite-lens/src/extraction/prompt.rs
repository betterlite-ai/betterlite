use super::{navigation::Navigation, storage::Summary};
use crate::analysis::primitives::{FeedbackRow, structured_response};
use crate::analysis::{AnalysisError, Extraction, ModelCall, Observation, SpanRead, TraceReview};
use crate::models::{
    execution::{Execution, ExecutionContent, TracePart},
    messages::{Claim, ModelRequest, Purpose},
    settings::Check,
};
use crate::trace_store::{CatalogRow, overview_content};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub(super) struct Context<'context> {
    pub claim: &'context Claim,
    pub execution: &'context Execution,
    pub page: &'context ExecutionContent,
    pub summary: &'context Summary,
    pub feedback: Vec<Vec<FeedbackRow>>,
}

#[derive(Serialize)]
struct Prompt<'prompt> {
    task: &'prompt str,
    navigation: &'prompt str,
    must_decide: bool,
    context: &'prompt str,
    checks: Vec<Check>,
    execution: &'prompt Execution,
    catalog_complete: bool,
    catalog_fields: [&'prompt str; 5],
    catalog: &'prompt [CatalogRow],
    task_and_outcome: Vec<TracePart>,
    read_evidence: &'prompt [TracePart],
    previous_observations: &'prompt [Observation],
    completed_read_count: usize,
    last_completed_read: Option<&'prompt SpanRead>,
    feedback: &'prompt [FeedbackRow],
    feedback_page: usize,
    feedback_pages: usize,
    response_schema: Schema,
}

#[derive(Deserialize, Serialize)]
struct Schema {
    #[serde(rename = "type", default, skip_serializing_if = "Option::is_none")]
    kind: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    properties: BTreeMap<String, Self>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    items: Option<Box<Self>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    required: Vec<String>,
    #[serde(rename = "anyOf", default, skip_serializing_if = "Vec::is_empty")]
    any_of: Vec<Self>,
    #[serde(rename = "maxItems", default, skip_serializing_if = "Option::is_none")]
    max_items: Option<usize>,
    #[serde(rename = "enum", default, skip_serializing_if = "Vec::is_empty")]
    choices: Vec<String>,
    #[serde(rename = "maxLength", default, skip_serializing_if = "Option::is_none")]
    max_length: Option<usize>,
    #[serde(rename = "minLength", default, skip_serializing_if = "Option::is_none")]
    min_length: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    minimum: Option<usize>,
    #[serde(
        rename = "additionalProperties",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    additional_properties: Option<bool>,
}

fn payload<'prompt>(
    context: &'prompt Context<'_>,
    catalog: &'prompt [CatalogRow],
    state: &'prompt Navigation,
) -> Result<Prompt<'prompt>, AnalysisError> {
    let root = context
        .summary
        .root
        .iter()
        .map(|part| {
            let mut excerpt = part.clone();
            excerpt.content = overview_content(part, context.summary.root_count);
            excerpt
        })
        .collect();
    Ok(Prompt {
        task: include_str!("instructions.txt"),
        navigation: "The current feedback page is already included. Only request a different feedback_page when feedback_pages>1. Zero feedback_pages means there is no feedback to consult. When must_decide=true, return final observations without further reads or navigation.",
        must_decide: state.must_decide,
        context: &context.claim.job.settings.context,
        checks: context.claim.job.settings.analysis_checks(),
        execution: context.execution,
        catalog_complete: context.page.next_cursor.is_none()
            && catalog.len() == context.summary.span_count,
        catalog_fields: ["span_id", "parent_span_id", "name", "kind", "preview"],
        catalog,
        task_and_outcome: root,
        read_evidence: state
            .additional
            .get(state.additional.len().saturating_sub(2)..)
            .unwrap_or_default(),
        previous_observations: &state.previous.observations,
        completed_read_count: state.reads.len(),
        last_completed_read: state.reads.last(),
        feedback: context
            .feedback
            .get(state.feedback_page)
            .map_or(&[], Vec::as_slice),
        feedback_page: state.feedback_page,
        feedback_pages: context.feedback.len(),
        response_schema: response_schema(state.must_decide)?,
    })
}

pub(super) async fn review(
    context: &Context<'_>,
    catalog: &[CatalogRow],
    state: &Navigation,
    model: &ModelCall,
) -> Result<TraceReview, AnalysisError> {
    let request = ModelRequest {
        prompt: serde_json::to_string(&payload(context, catalog, state)?)?,
        purpose: Purpose::Extract,
    };
    if state.must_decide {
        let result: Extraction = structured_response(request, |sent| model(sent), |_| None).await?;
        Ok(TraceReview {
            cannot_assess: result.cannot_assess,
            observations: result.observations,
            feedback_page: None,
            reads: Vec::new(),
        })
    } else {
        structured_response(request, |sent| model(sent), |_| None).await
    }
}

fn response_schema(must_decide: bool) -> Result<Schema, AnalysisError> {
    let mut schema: Schema = serde_json::from_str(include_str!("schema.json"))?;
    if must_decide {
        schema.properties.remove("reads");
        schema.properties.remove("feedback_page");
    }
    Ok(schema)
}
