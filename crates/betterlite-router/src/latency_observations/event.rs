use rust_decimal::Decimal;

use crate::cooldown_decisions::failure::FailureClass;

// one metadata channel as litellm_params carries it: absent, or present with the model group it holds
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Channel<'request> {
    #[default]
    Absent,
    Present(Option<&'request str>),
}

// custom_logger._select_metadata_field reads litellm_metadata whenever the request carries it, and metadata otherwise
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RequestChannels<'request> {
    pub litellm_metadata: Channel<'request>,
    pub metadata: Channel<'request>,
}

impl<'request> RequestChannels<'request> {
    #[must_use]
    pub const fn model_group(&self) -> Option<&'request str> {
        match self.litellm_metadata {
            Channel::Present(group) => group,
            Channel::Absent => match self.metadata {
                Channel::Present(group) => group,
                Channel::Absent => None,
            },
        }
    }
}

// ModelResponse.usage as the success callback reads it
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TokenUsage {
    pub completion_tokens: i64,
    pub total_tokens: i64,
}

// log_success_event and async_log_success_event: the logged request and the times it reported
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SuccessEvent<'event> {
    pub call_type: Option<&'event str>,
    pub channels: RequestChannels<'event>,
    pub deployment_id: Option<&'event str>,
    pub stream: bool,
    pub start_time: Decimal,
    pub end_time: Decimal,
    pub completion_start_time: Option<Decimal>,
    pub usage: Option<TokenUsage>,
}

// async_log_failure_event only acts on a litellm.Timeout, which is the Timeout class of betterlite's failures
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FailureEvent<'event> {
    pub call_type: Option<&'event str>,
    pub channels: RequestChannels<'event>,
    pub deployment_id: Option<&'event str>,
    pub failure: FailureClass,
}
