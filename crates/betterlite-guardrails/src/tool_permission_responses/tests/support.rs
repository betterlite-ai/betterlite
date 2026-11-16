use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use betterlite_streaming::stream_usage_assembly::UsageTokenCounter;
use betterlite_streaming::stream_usage_assembly::wire::Usage;
use betterlite_streaming::streamed_response_assembly::cost::{
    CostError, CostInput, StreamCostCalculator,
};
use betterlite_tokenizer::count::error::CountError;
use betterlite_tokenizer::count::request::Message;
use rust_decimal::Decimal;

use super::super::stream_services::StreamServices;
use crate::guardrail_initializers::tool_permission::{
    ToolDefaultAction, ToolDisallowedAction, ToolPermissionArgs,
};
use crate::guardrail_message_scope::Object;
use crate::tool_permission::ToolPermissionGuardrail;

pub(crate) const GUARDRAIL_NAME: &str = "tools-check";

#[must_use]
pub(crate) fn text(value: &str) -> GuardrailValue {
    GuardrailValue::Text(value.to_owned())
}

#[must_use]
pub(crate) fn json(source: &str) -> GuardrailValue {
    serde_json::from_str(source).expect("valid guardrail json")
}

#[must_use]
pub(crate) fn object(source: &str) -> Object {
    serde_json::from_str(source).expect("valid guardrail object")
}

#[must_use]
pub(crate) fn rule(id: &str, decision: &str, tool_name: &str) -> BTreeMap<String, GuardrailValue> {
    BTreeMap::from([
        ("id".to_owned(), text(id)),
        ("decision".to_owned(), text(decision)),
        ("tool_name".to_owned(), text(tool_name)),
    ])
}

fn args(
    rules: Vec<BTreeMap<String, GuardrailValue>>,
    action: ToolDisallowedAction,
    default_on: bool,
) -> ToolPermissionArgs {
    ToolPermissionArgs {
        name: GUARDRAIL_NAME.to_owned(),
        event_hook: LitellmParamsMode::One("post_call".to_owned()),
        rules: Some(rules),
        default_action: ToolDefaultAction::Deny,
        on_disallowed_action: action,
        default_on,
        violation_message_template: None,
    }
}

#[must_use]
pub(crate) fn guardrail(rules: Vec<BTreeMap<String, GuardrailValue>>) -> ToolPermissionGuardrail {
    ToolPermissionGuardrail::new(args(rules, ToolDisallowedAction::Block, true))
        .expect("guardrail builds")
}

#[must_use]
pub(crate) fn rewrite_guardrail(
    rules: Vec<BTreeMap<String, GuardrailValue>>,
) -> ToolPermissionGuardrail {
    ToolPermissionGuardrail::new(args(rules, ToolDisallowedAction::Rewrite, true))
        .expect("guardrail builds")
}

#[must_use]
pub(crate) fn skipping_guardrail() -> ToolPermissionGuardrail {
    ToolPermissionGuardrail::new(args(Vec::new(), ToolDisallowedAction::Block, false))
        .expect("guardrail builds")
}

struct NoCosts;

impl StreamCostCalculator for NoCosts {
    fn completion_cost(
        &self,
        _input: &CostInput<'_>,
        _provider: Option<&str>,
    ) -> Result<Option<Decimal>, CostError> {
        Ok(None)
    }

    fn prompt_completion_cost(
        &self,
        _model: &str,
        _usage: &Usage,
    ) -> Result<(Decimal, Decimal), CostError> {
        Ok((Decimal::ZERO, Decimal::ZERO))
    }
}

struct NoCounter;

#[async_trait]
impl UsageTokenCounter for NoCounter {
    async fn count_response_text(&self, _model: &str, _text: &str) -> Result<i64, CountError> {
        Ok(0)
    }

    async fn count_text(&self, _text: &str) -> Result<i64, CountError> {
        Ok(0)
    }

    async fn count_messages(
        &self,
        _model: &str,
        _messages: Option<&[Message]>,
    ) -> Result<i64, CountError> {
        Ok(0)
    }
}

static NO_COSTS: NoCosts = NoCosts;
static NO_COUNTER: NoCounter = NoCounter;

#[must_use]
pub(crate) fn services() -> StreamServices<'static> {
    StreamServices {
        costs: &NO_COSTS,
        counter: &NO_COUNTER,
        redactor: Arc::new(
            Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("redactor patterns compile"),
        ),
        now: 1_700_000_000,
    }
}
