use std::sync::Arc;

use async_trait::async_trait;
use betterlite_anthropic::stream_state::state::IdSource;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_guardrails::guardrail_initializers::tool_permission::ToolPermissionArgs;
use betterlite_guardrails::tool_permission::ToolPermissionGuardrail;
use betterlite_guardrails::tool_permission::config::ToolPermissionConfigModel;
use betterlite_guardrails::tool_permission_responses::stream_services::StreamServices;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use betterlite_streaming::stream_usage_assembly::UsageTokenCounter;
use betterlite_streaming::stream_usage_assembly::wire::Usage;
use betterlite_streaming::streamed_response_assembly::cost::{
    CostError, CostInput, StreamCostCalculator,
};
use betterlite_tokenizer::count::error::CountError;
use betterlite_tokenizer::count::request::Message;
use rust_decimal::Decimal;
use tokio::runtime::{Builder, Runtime};
use uuid::Uuid;

use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};

use super::{GUARDRAIL_NAME, Input};

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

struct FixedIds;

impl IdSource for FixedIds {
    fn next_id(&self) -> Uuid {
        Uuid::from_u128(0x3813)
    }
}

static FIXED_IDS: FixedIds = FixedIds;

pub(crate) fn failure(class: &str, message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

pub(crate) fn build_guardrail(input: &Input) -> Result<ToolPermissionGuardrail, Box<HelperError>> {
    let settings = ToolPermissionConfigModel {
        optional_params: None,
        rules: None,
        default_action: input.default_action.clone(),
        on_disallowed_action: input.on_disallowed_action.clone(),
    }
    .resolve()
    .or_value_error()?;
    let args = ToolPermissionArgs {
        name: GUARDRAIL_NAME.to_owned(),
        event_hook: LitellmParamsMode::One(input.event_hook.clone()),
        rules: Some(input.rules.clone()),
        default_action: settings.default_action,
        on_disallowed_action: settings.on_disallowed_action,
        default_on: input.default_on,
        violation_message_template: input.violation_message_template.clone(),
    };
    ToolPermissionGuardrail::new(args).or_value_error()
}

pub(crate) fn services(now: i64) -> Result<StreamServices<'static>, Box<HelperError>> {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).or_value_error()?;
    Ok(StreamServices {
        costs: &NO_COSTS,
        counter: &NO_COUNTER,
        redactor: Arc::new(redactor),
        ids: &FIXED_IDS,
        now,
    })
}

pub(crate) fn runtime() -> Result<Runtime, Box<HelperError>> {
    Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| failure("RuntimeError", error.to_string()))
}
