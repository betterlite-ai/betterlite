mod beta_fields;
mod computer_versions;
mod context_filter;
mod output_config;
mod tool_names;

use super::support::{Fixture, Processed, Request, Session};

const MODEL_COST: &str = r#"{
    "anthropic.fixture-adaptive": {"litellm_provider": "bedrock", "supports_adaptive_thinking": true},
    "anthropic.fixture-plain": {"litellm_provider": "bedrock", "supports_adaptive_thinking": false}
}"#;

pub(super) const ADAPTIVE_MODEL: &str = "anthropic.fixture-adaptive";
pub(super) const PLAIN_MODEL: &str = "anthropic.fixture-plain";
pub(super) const FOREIGN_MODEL: &str = "meta.llama3-v1:0";

pub(super) fn prepare(
    session: &mut Session<'_>,
    model: &str,
    tools: &str,
    headers: Option<&str>,
    params: &str,
) -> Processed {
    session
        .process(&Request::new(model, tools, headers, params))
        .expect("the request prepares")
}

pub(super) fn anthropic_beta_fields() {
    let fixture = Fixture::new(MODEL_COST);
    let mut session = fixture.session();
    beta_fields::client_headers(&mut session);
    beta_fields::eager_added(&mut session);
    beta_fields::eager_deduplicated(&mut session);
    beta_fields::eager_malformed(&mut session);
    beta_fields::effort_values(&mut session);
    beta_fields::effort_without_effect(&mut session);
    beta_fields::parameter_writing(&mut session);
}

pub(super) fn tool_name_map() {
    let fixture = Fixture::new(MODEL_COST);
    let mut session = fixture.session();
    tool_names::partitioning(&mut session);
    tool_names::tool_search_dropped(&mut session);
    tool_names::conversion(&mut session);
    tool_names::schemas(&mut session);
    tool_names::failures(&mut session);
}

pub(super) fn output_config_tools() {
    let fixture = Fixture::new(MODEL_COST);
    let mut session = fixture.session();
    output_config::preserved(&mut session);
    output_config::unrelated_params(&mut session);
}

pub(super) fn computer_use_beta_versions() {
    let fixture = Fixture::new(MODEL_COST);
    let mut session = fixture.session();
    computer_versions::version_markers();
    computer_versions::older_versions();
    computer_versions::request(&mut session);
    computer_versions::foreign_base(&mut session);
}

pub(super) fn context_management_filter() {
    let fixture = Fixture::new(MODEL_COST);
    let mut session = fixture.session();
    context_filter::removed_foreign(&mut session);
    context_filter::removed_shapes(&mut session);
    context_filter::retained_edits(&mut session);
    context_filter::retained_non_dict_edits(&mut session);
    context_filter::retained_compact_fields(&mut session);
}
