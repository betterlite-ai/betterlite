use rust_decimal::Decimal;

// async_log_success_event tells these call types apart; every other call type spends once per tier
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CallType<'event> {
    Absent,
    ResponsesWebsocket,
    Realtime,
    Other(&'event str),
}

impl<'event> CallType<'event> {
    // the standard_logging_object call_type async_log_success_event reads, as the set the limiter knows
    #[must_use]
    pub fn parse(call_type: Option<&'event str>) -> Self {
        match call_type {
            Some("_aresponses_websocket") => Self::ResponsesWebsocket,
            Some("_arealtime") => Self::Realtime,
            Some(other) => Self::Other(other),
            None => Self::Absent,
        }
    }
}

// the standard_logging_object fields and request tags async_log_success_event reads; B321 answers the tags
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SuccessEvent<'event> {
    pub call_type: CallType<'event>,
    pub response_cost: Decimal,
    pub model_id: Option<&'event str>,
    pub custom_llm_provider: Option<&'event str>,
    pub tags: &'event [String],
}
