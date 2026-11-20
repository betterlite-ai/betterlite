use std::collections::BTreeMap;

use betterlite_config::sections::content_category::loading::CategoryLoadResult;
use betterlite_config::sections::content_filter_records::{BlockedWord, ContentFilterPattern};
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_guardrails::content_filter_actions::ContentFilterSource;
use betterlite_guardrails::guardrail_base::GuardrailBase;
use betterlite_guardrails::guardrail_base::settings::GuardrailSettings;
use betterlite_guardrails::guardrail_base::settings::GuardrailSwitches;
use betterlite_guardrails::pattern_matching::{CompiledPattern, compile};
use serde::Deserialize;
use tokio::runtime::{Builder, Runtime};

use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

mod operations;
mod projection;

const GUARDRAIL_NAME: &str = "content-filter-inputs";
const PATTERN_FORMAT: &str = "[{pattern_name}_REDACTED]";
const KEYWORD_TAG: &str = "[KEYWORD_REDACTED]";

#[derive(Debug, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case")]
pub(crate) enum Input {
    Images(ImagesInput),
    Competitor(CompetitorInput),
    ToolCalls(ToolCallsInput),
    Mcp(McpInput),
}

#[derive(Debug, Deserialize)]
pub(crate) struct ImagesInput {
    patterns: Vec<ContentFilterPattern>,
    blocked_words: Vec<BlockedWord>,
    images: Vec<String>,
    image_model: String,
    image_description: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct CompetitorInput {
    text: String,
    model: String,
    config: CompetitorConfigInput,
    patterns: Vec<ContentFilterPattern>,
    blocked_words: Vec<BlockedWord>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct CompetitorConfigInput {
    brand_self: Vec<String>,
    competitors: Vec<String>,
    locations: Vec<String>,
    #[serde(default)]
    policy: BTreeMap<String, String>,
    threshold_high: Option<f64>,
    threshold_medium: Option<f64>,
    threshold_low: Option<f64>,
    refuse_message_template: Option<String>,
    reframe_message_template: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ToolCallsInput {
    patterns: Vec<ContentFilterPattern>,
    blocked_words: Vec<BlockedWord>,
    tool_calls: Vec<GuardrailValue>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct McpInput {
    patterns: Vec<ContentFilterPattern>,
    blocked_words: Vec<BlockedWord>,
    request_data: BTreeMap<String, GuardrailValue>,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::Images(images) => operations::run_images(images),
            Input::Competitor(competitor) => operations::run_competitor(competitor),
            Input::ToolCalls(tool_calls) => operations::run_tool_calls(tool_calls),
            Input::Mcp(mcp) => operations::run_mcp(mcp),
        }
    }
}

struct Fixture {
    categories: CategoryLoadResult,
    patterns: Vec<CompiledPattern>,
    blocked_words: Vec<BlockedWord>,
}

impl Fixture {
    fn new(
        configs: &[ContentFilterPattern],
        blocked_words: &[BlockedWord],
    ) -> Result<Self, Box<HelperError>> {
        let patterns = configs
            .iter()
            .map(|config| {
                compile(config).map_err(|source| failure("PatternError", source.to_string()))
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            categories: CategoryLoadResult::default(),
            patterns,
            blocked_words: blocked_words.to_vec(),
        })
    }

    fn source(&self) -> ContentFilterSource<'_> {
        ContentFilterSource {
            categories: &self.categories,
            patterns: &self.patterns,
            blocked_words: &self.blocked_words,
            pattern_redaction_format: PATTERN_FORMAT,
            keyword_redaction_tag: KEYWORD_TAG,
        }
    }
}

fn guardrail(event_hook: &str) -> GuardrailBase {
    GuardrailBase {
        settings: GuardrailSettings {
            guardrail_name: Some(GUARDRAIL_NAME.to_owned()),
            supported_event_hooks: None,
            event_hook: Some(LitellmParamsMode::One(event_hook.to_owned())),
            default_on: false,
            mask_request_content: false,
            mask_response_content: false,
            violation_message_template: None,
            end_session_after_n_fails: None,
            on_violation: None,
            realtime_violation_message: None,
            on_sensitive_data: None,
            sensitive_data_route_to_model: None,
            sticky_session_routing: true,
            run_in_parallel: false,
            scan_raw_request: false,
            only_scan_new_messages: false,
            timeout: None,
            switches: GuardrailSwitches::default(),
        },
        mode: None,
    }
}

fn runtime() -> Result<Runtime, Box<HelperError>> {
    Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|source| failure("RuntimeError", source.to_string()))
}

fn failure(class: &str, message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}
