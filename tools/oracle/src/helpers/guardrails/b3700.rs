use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::time::Duration;

use betterlite_config::sections::content_category::Settings as CategorySettings;
use betterlite_config::sections::content_category::loading::{CategoryLoadResult, load_categories};
use betterlite_config::sections::content_filter_records::{
    BlockedWord, ContentFilterAction, ContentFilterCategoryConfig, ContentFilterPattern,
    ContentFilterSeverity,
};
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_guardrails::content_filter_actions::ContentFilterSource;
use betterlite_guardrails::content_filter_streaming::hook::{
    StreamFilterCall, StreamFilterSettings, stream_filter_hook,
};
use betterlite_guardrails::pattern_matching::{CompiledPattern, compile};
use serde::Deserialize;
use tokio::runtime::Builder;
use tokio::sync::mpsc;
use tokio::time::timeout;

use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

mod projection;

const GUARDRAIL_NAME: &str = "stream-filter";
const DEFAULT_PATTERN_FORMAT: &str = "[{pattern_name}_REDACTED]";
const DEFAULT_KEYWORD_TAG: &str = "[KEYWORD_REDACTED]";
const HOOK_TIMEOUT: Duration = Duration::from_secs(30);

pub(crate) struct Op;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    patterns: Vec<ContentFilterPattern>,
    blocked_words: Vec<BlockedWord>,
    categories: Vec<CategoryInput>,
    guardrail_id: Option<String>,
    policy_template: Option<String>,
    pattern_redaction_format: Option<String>,
    keyword_redaction_tag: Option<String>,
    items: Vec<GuardrailValue>,
    close_after: Option<usize>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CategoryInput {
    category: String,
    action: ContentFilterAction,
    severity_threshold: ContentFilterSeverity,
    file_text: String,
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let directory =
            tempfile::tempdir().map_err(|source| failure("TempDirError", source.to_string()))?;
        let categories = load_input_categories(directory.path(), &input.categories)?;
        let patterns = compile_patterns(&input.patterns)?;
        let settings = stream_settings(input);
        let source = ContentFilterSource {
            categories: &categories,
            patterns: &patterns,
            blocked_words: &input.blocked_words,
            pattern_redaction_format: &settings.pattern_redaction_format,
            keyword_redaction_tag: &settings.keyword_redaction_tag,
        };
        let runtime = Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|error| failure("RuntimeError", error.to_string()))?;
        runtime.block_on(stream_output(input, &settings, &source))
    }
}

// The case carries the category file body, so both sides write the same file and load it
fn load_input_categories(
    directory: &Path,
    inputs: &[CategoryInput],
) -> Result<CategoryLoadResult, Box<HelperError>> {
    let mut configs = Vec::with_capacity(inputs.len());
    for (position, category) in inputs.iter().enumerate() {
        let path = directory.join(format!("category_{position}.yaml"));
        fs::write(&path, &category.file_text)
            .map_err(|source| failure("CategoryFileError", source.to_string()))?;
        configs.push(ContentFilterCategoryConfig {
            category: category.category.clone(),
            enabled: true,
            action: category.action,
            severity_threshold: category.severity_threshold,
            category_file: Some(path.display().to_string()),
        });
    }
    // The category files sit in the helper's temp folder, the one root it grants the loader
    Ok(load_categories(
        &configs,
        &[directory.to_path_buf()],
        Path::new("."),
        &CategorySettings {
            allow_external_paths: false,
        },
    ))
}

fn compile_patterns(
    configs: &[ContentFilterPattern],
) -> Result<Vec<CompiledPattern>, Box<HelperError>> {
    let mut compiled = Vec::with_capacity(configs.len());
    for config in configs {
        let pattern =
            compile(config).map_err(|source| failure("PatternError", source.to_string()))?;
        compiled.push(pattern);
    }
    Ok(compiled)
}

fn stream_settings(input: &Input) -> StreamFilterSettings {
    StreamFilterSettings {
        guardrail_name: GUARDRAIL_NAME.to_owned(),
        event_hook: LitellmParamsMode::One("post_call".to_owned()),
        guardrail_id: input.guardrail_id.clone(),
        policy_template: input.policy_template.clone(),
        pattern_redaction_format: input
            .pattern_redaction_format
            .clone()
            .unwrap_or_else(|| DEFAULT_PATTERN_FORMAT.to_owned()),
        keyword_redaction_tag: input
            .keyword_redaction_tag
            .clone()
            .unwrap_or_else(|| DEFAULT_KEYWORD_TAG.to_owned()),
    }
}

#[expect(
    clippy::map_err_ignore,
    reason = "a timeout only says the time ran out, and the answer names it TimeoutError"
)]
async fn stream_output(
    input: &Input,
    settings: &StreamFilterSettings,
    source: &ContentFilterSource<'_>,
) -> HelperResult {
    let (requests, response) = mpsc::channel(input.items.len().max(1));
    let (output, relayed) = mpsc::channel(1);
    let feeder = feed(&input.items, requests);
    let consumer = consume(relayed, input.close_after);
    let mut request_data = BTreeMap::new();
    let (outcome, (), chunks) = {
        let mut call = StreamFilterCall {
            user_api_key_dict: None,
            request_data: &mut request_data,
            log_target: None,
        };
        let hook = stream_filter_hook(response, output, settings, source, &mut call);
        timeout(
            HOOK_TIMEOUT,
            async move { tokio::join!(hook, feeder, consumer) },
        )
        .await
        .map_err(|_| {
            failure(
                "TimeoutError",
                "the streaming hook did not finish".to_owned(),
            )
        })?
    };
    let projected = projection::chunks(&chunks);
    let record = projection::record(&request_data);
    let error = projection::error(&outcome)?;
    Ok(serde_json::json!({
        "chunks": projected,
        "record": record,
        "error": error,
    }))
}

async fn feed(items: &[GuardrailValue], sender: mpsc::Sender<GuardrailValue>) {
    for item in items {
        if sender.send(item.clone()).await.is_err() {
            break;
        }
    }
}

// Owning the receiver is what models a client that stops reading: dropping it breaks the hook's send
async fn consume(
    mut relayed: mpsc::Receiver<GuardrailValue>,
    close_after: Option<usize>,
) -> Vec<GuardrailValue> {
    let limit = close_after.unwrap_or(usize::MAX);
    let mut chunks = Vec::new();
    while let Some(item) = relayed.recv().await {
        chunks.push(item);
        if chunks.len() >= limit {
            break;
        }
    }
    chunks
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
