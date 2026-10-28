use crate::context::SessionContext;
use crate::options::{CodexOptions, ReasoningEffort};

use super::config::toml_string;

pub(super) fn feature_overrides<S>(
    context: &SessionContext<S>,
    options: &CodexOptions,
) -> Vec<String> {
    let mut values = vec![format!(
        "web_search={}",
        if options.web_search {
            "live"
        } else {
            "disabled"
        }
    )];
    if let Some(effort) = options.reasoning_effort {
        let value = match effort {
            ReasoningEffort::Low => "low",
            ReasoningEffort::Medium => "medium",
            ReasoningEffort::High => "high",
            ReasoningEffort::Xhigh => "xhigh",
        };
        values.push(format!("model_reasoning_effort={value}"));
        values.push("model_reasoning_summary=auto".to_owned());
        values.push("model_supports_reasoning_summaries=true".to_owned());
    }
    if let Some(instructions) = context.settings.instructions.as_ref()
        && let Ok(encoded) = toml_string(instructions)
    {
        values.push(format!("developer_instructions={encoded}"));
    }
    values
}
