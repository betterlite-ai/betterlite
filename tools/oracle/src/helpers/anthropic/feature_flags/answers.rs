use betterlite_anthropic::anthropic_capability_catalog::CatalogSources;
use betterlite_anthropic::feature_flags::beta::{
    computer_tool_beta_header, tool_search_beta_header, user_beta_headers,
};
use betterlite_anthropic::feature_flags::billing::{billing_fields, is_one_shot_subagent_request};
use betterlite_anthropic::feature_flags::cache::supports_cache_control;
use betterlite_anthropic::feature_flags::compaction::requires_compaction_beta;
use betterlite_anthropic::feature_flags::messages::{
    has_output_config, has_thinking_display_updates, has_tool_change, is_cache_control_set,
    is_file_id_used, is_mcp_server_used, is_pdf_used,
};
use betterlite_anthropic::feature_flags::tools::{
    computer_tool_version, eager_input_streaming_flag, is_code_execution_tool_used,
    is_container_with_skills_used, is_eager_input_streaming_used, is_native_web_search_tool,
    is_tool_search_used, is_web_search_tool_used, uses_input_examples, uses_programmatic_calls,
};
use betterlite_models::lemonade_model_metadata::ModelInfoClient;
use betterlite_models::model_capability_predicates::CapabilityEnvironment;
use betterlite_models::model_metadata_resolution::dynamic::DynamicMetadataSource;
use serde::Deserialize;
use serde_json::{Value, json};

use super::calls::beta::Beta;
use super::calls::{
    BillingFields, CacheControl, CacheControlSet, Compaction, Computer, Conversation, EagerFlag,
    EagerUsed, FileIds, OneShot, Pdf, Skills, Tools,
};
use super::fixture::Fixture;

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(super) enum Call {
    OneShot(OneShot),
    BillingFields(BillingFields),
    Compaction(Compaction),
    CacheControl(CacheControl),
    CacheControlSet(CacheControlSet),
    FileIds(FileIds),
    Pdf(Pdf),
    Tools(Tools),
    EagerFlag(EagerFlag),
    EagerUsed(EagerUsed),
    Computer(Computer),
    Skills(Skills),
    Conversation(Conversation),
    Beta(Beta),
}

impl Call {
    pub(super) async fn answer<Source, Transport>(
        &self,
        environment: &mut CapabilityEnvironment<'_, Source, Transport>,
        catalogs: &CatalogSources<'_>,
    ) -> Value
    where
        Source: DynamicMetadataSource,
        Transport: ModelInfoClient,
    {
        match self {
            Self::OneShot(case) => one_shot_answer(case),
            Self::BillingFields(case) => billing_fields_answer(case),
            Self::Compaction(case) => compaction_answer(case),
            Self::CacheControl(case) => cache_control_answer(case, environment).await,
            Self::CacheControlSet(case) => {
                json!({"cache_control": is_cache_control_set(&case.messages)})
            }
            Self::FileIds(case) => file_ids_answer(case),
            Self::Pdf(case) => pdf_answer(case),
            Self::Tools(case) => tools_answer(case),
            Self::EagerFlag(case) => eager_flag_answer(case),
            Self::EagerUsed(case) => eager_used_answer(case),
            Self::Computer(case) => computer_answer(case),
            Self::Skills(case) => skills_answer(case),
            Self::Conversation(case) => conversation_answer(case),
            Self::Beta(case) => case.answer(environment, catalogs).await,
        }
    }
}

fn one_shot_answer(case: &OneShot) -> Value {
    json!({
        "one_shot": is_one_shot_subagent_request(
            &case.messages,
            &case.system,
            case.tools.as_deref().unwrap_or_default(),
            case.user_agent.as_deref(),
        ),
    })
}

fn billing_fields_answer(case: &BillingFields) -> Value {
    let fields: Vec<Value> = case
        .texts
        .iter()
        .map(|text| json!(billing_fields(text)))
        .collect();
    json!({"fields": fields})
}

fn compaction_answer(case: &Compaction) -> Value {
    json!({
        "native": requires_compaction_beta(&case.provider, &case.params, &case.messages),
    })
}

async fn cache_control_answer<Source, Transport>(
    case: &CacheControl,
    environment: &mut CapabilityEnvironment<'_, Source, Transport>,
) -> Value
where
    Source: DynamicMetadataSource,
    Transport: ModelInfoClient,
{
    let supported = supports_cache_control(
        &case.model,
        case.custom_llm_provider.as_deref(),
        environment,
    )
    .await;
    json!({"supported": supported})
}

fn file_ids_answer(case: &FileIds) -> Value {
    // LiteLLM raises on a content block with no type; the Python adapter reports that as no answer
    is_file_id_used(&case.messages).map_or_else(
        |_| json!({"file_id": null}),
        |used| json!({"file_id": used}),
    )
}

fn pdf_answer(case: &Pdf) -> Value {
    json!({
        "pdf": is_pdf_used(&case.messages),
    })
}

fn tools_answer(case: &Tools) -> Value {
    let tools = case.tools.as_deref().unwrap_or_default();
    let mut native = Vec::new();
    if !tools.is_empty() {
        native.extend(tools.iter().map(is_native_web_search_tool));
    }
    json!({
        "tool_search": is_tool_search_used(tools),
        "programmatic": uses_programmatic_calls(tools),
        "input_examples": uses_input_examples(tools),
        "native": native,
    })
}

fn eager_flag_answer(case: &EagerFlag) -> Value {
    match eager_input_streaming_flag(&case.tool) {
        Ok(flag) => json!({"flag": flag}),
        Err(error) => json!({"error": {"status": error.status_code()}}),
    }
}

fn eager_used_answer(case: &EagerUsed) -> Value {
    let tools = case.tools.as_deref().unwrap_or_default();
    match is_eager_input_streaming_used(tools) {
        Ok(used) => json!({"used": used}),
        Err(error) => json!({"error": {"status": error.status_code()}}),
    }
}

fn computer_answer(case: &Computer) -> Value {
    let tools = case.tools.as_deref().unwrap_or_default();
    let mut betas = Vec::with_capacity(case.versions.len());
    for version in &case.versions {
        betas.push(computer_tool_beta_header(version));
    }
    json!({
        "computer_tool": tools.iter().find_map(computer_tool_version),
        "betas": betas,
        "web_search": is_web_search_tool_used(tools),
    })
}

fn skills_answer(case: &Skills) -> Value {
    let user_betas: Vec<Option<Vec<String>>> = case
        .headers
        .iter()
        .map(|header| user_beta_headers(header.as_deref()))
        .collect();
    let tools = case.tools.as_deref().unwrap_or_default();
    let search_betas: Vec<&'static str> = case
        .providers
        .iter()
        .map(|provider| tool_search_beta_header(provider))
        .collect();
    json!({
        "code_execution": is_code_execution_tool_used(tools),
        "container_skills": is_container_with_skills_used(&case.params),
        "user_betas": user_betas,
        "tool_search_betas": search_betas,
    })
}

fn conversation_answer(case: &Conversation) -> Value {
    let mcp = is_mcp_server_used(case.mcp_servers.as_deref());
    json!({
        "thinking_display": has_thinking_display_updates(Some(&case.thinking)),
        "tool_change": has_tool_change(&case.messages),
        "output_config": has_output_config(&case.messages),
        "mcp": mcp,
    })
}

pub(super) async fn run(calls: &[Call], fixture: &Fixture) -> crate::helpers::HelperResult {
    let mut session = fixture.session();
    let mut answers = Vec::with_capacity(calls.len());
    for call in calls {
        let mut environment = session.environment();
        answers.push(call.answer(&mut environment, &fixture.catalogs()).await);
    }
    Ok(serde_json::json!({ "calls": answers }))
}
