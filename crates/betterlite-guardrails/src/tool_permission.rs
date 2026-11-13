use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;

use crate::guardrail_initializers::tool_permission::ToolPermissionArgs;

pub mod config;
pub mod error;
pub mod matching;
pub mod rule;
mod tests;
pub mod types;

pub const GUARDRAIL_NAME: &str = "tool_permission";
pub const SUPPORTED_EVENT_HOOKS: [GuardrailEventHooks; 2] =
    [GuardrailEventHooks::PreCall, GuardrailEventHooks::PostCall];
pub const DEFAULT_MAX_RECURSE_DEPTH: usize = 100;

use config::default_action_text;
use error::ToolPermissionError;
use matching::CompiledRules;
use rule::ToolPermissionRule;

#[derive(Debug, Clone)]
pub struct ToolPermissionGuardrail {
    pub args: ToolPermissionArgs,
    pub rules: Vec<ToolPermissionRule>,
    pub compiled: CompiledRules,
}

impl ToolPermissionGuardrail {
    pub fn new(args: ToolPermissionArgs) -> Result<Self, ToolPermissionError> {
        let (rules, compiled) = compile_rules(args.rules.as_deref())?;
        let guardrail = Self {
            args,
            rules,
            compiled,
        };
        tracing::debug!(
            rules = guardrail.rules.len(),
            default_action = default_action_text(guardrail.args.default_action),
            "Tool Permission Guardrail initialized"
        );
        Ok(guardrail)
    }

    // LiteLLM's _load_rules swaps in the new state only after every pattern compiles
    pub fn load_rules(
        &mut self,
        rules: Option<&[BTreeMap<String, GuardrailValue>]>,
    ) -> Result<(), ToolPermissionError> {
        let (parsed, compiled) = compile_rules(rules)?;
        self.rules = parsed;
        self.compiled = compiled;
        Ok(())
    }
}

fn compile_rules(
    rules: Option<&[BTreeMap<String, GuardrailValue>]>,
) -> Result<(Vec<ToolPermissionRule>, CompiledRules), ToolPermissionError> {
    let mut parsed = Vec::new();
    let mut compiled = CompiledRules::default();
    for fields in rules.unwrap_or_default() {
        let rule = ToolPermissionRule::from_fields(fields)?;
        compiled.compile_rule(&rule)?;
        parsed.push(rule);
    }
    Ok((parsed, compiled))
}
