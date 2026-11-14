use std::collections::BTreeMap;
use std::fmt::Debug;
use std::sync::{Arc, Mutex};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use betterlite_core::skills::content::files::extract_all_files;
// ast-grep-ignore: no-long-names
use betterlite_core::skills::content::inject::inject_skill_content_to_messages;
use betterlite_core::skills::content::prompt::extract_skill_content;
// ast-grep-ignore: no-long-names
use betterlite_core::skills::content::tools::convert_skill_to_anthropic_tool;
use betterlite_core::skills::content::tools::convert_skill_to_tool;
// ast-grep-ignore: no-long-names
use betterlite_core::skills::content::tools::get_litellm_code_execution_tool;
// ast-grep-ignore: no-long-names
use betterlite_core::skills::content::tools::get_litellm_code_execution_tool_anthropic;
use betterlite_db::managed_resource_rows::skills::SkillTable;
use betterlite_prompts::arguments::json::Object;
use serde::Deserialize;
use serde_json::json;
use tracing::field::{Field, Visit};
use tracing::subscriber::with_default;
use tracing::{Event, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};

use crate::helpers::{Helper, HelperResult, OrHelperError};

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    ExtractContent {
        skill: SkillTable,
    },
    ExtractFiles {
        skill: SkillTable,
    },
    ConvertTool {
        skill: SkillTable,
    },
    ConvertAnthropicTool {
        skill: SkillTable,
    },
    Inject {
        data: Object,
        contents: Vec<String>,
        use_anthropic_format: bool,
    },
    ExecutionTool,
    ExecutionToolAnthropic,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let captured = Warnings::default();
        let subscriber = tracing_subscriber::registry().with(captured.clone());
        let result = with_default(subscriber, || answer(input))?;
        Ok(json!({"result": result, "warnings": captured.messages()}))
    }
}

fn answer(input: &Input) -> HelperResult {
    match input {
        Input::ExtractContent { skill } => Ok(json!({"content": extract_skill_content(skill)})),
        Input::ExtractFiles { skill } => {
            let files: BTreeMap<&str, String> = extract_all_files(skill)
                .iter()
                .map(|(name, data)| (name.as_str(), STANDARD.encode(data)))
                .collect();
            Ok(json!({"files": files}))
        }
        Input::ConvertTool { skill } => {
            let tool = serde_json::to_value(convert_skill_to_tool(skill)).or_value_error()?;
            Ok(json!({"tool": tool}))
        }
        Input::ConvertAnthropicTool { skill } => {
            // ast-grep-ignore: no-long-names
            let converted = convert_skill_to_anthropic_tool(skill);
            let tool = serde_json::to_value(converted).or_value_error()?;
            Ok(json!({"tool": tool}))
        }
        Input::Inject {
            data,
            contents,
            use_anthropic_format,
        } => {
            let mut edited = data.clone();
            // ast-grep-ignore: no-long-names
            inject_skill_content_to_messages(&mut edited, contents, *use_anthropic_format)
                .or_value_error()?;
            Ok(json!({"data": edited}))
        }
        Input::ExecutionTool => {
            // ast-grep-ignore: no-long-names
            let tool = serde_json::to_value(get_litellm_code_execution_tool()).or_value_error()?;
            Ok(json!({"tool": tool}))
        }
        Input::ExecutionToolAnthropic => {
            // ast-grep-ignore: no-long-names
            let tool = serde_json::to_value(get_litellm_code_execution_tool_anthropic())
                .or_value_error()?;
            Ok(json!({"tool": tool}))
        }
    }
}

#[derive(Clone, Default)]
struct Warnings(Arc<Mutex<Vec<String>>>);

impl Warnings {
    fn messages(&self) -> Vec<String> {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }
}

#[derive(Default)]
struct MessageField(String);

impl Visit for MessageField {
    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?}");
        }
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.0 = value.to_owned();
        }
    }
}

impl<Inner: Subscriber> Layer<Inner> for Warnings {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, Inner>) {
        if *event.metadata().level() != tracing::Level::WARN {
            return;
        }
        let mut field = MessageField::default();
        event.record(&mut field);
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(field.0);
    }
}
