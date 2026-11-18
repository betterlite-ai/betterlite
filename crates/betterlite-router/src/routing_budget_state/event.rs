use rust_decimal::Decimal;

// the standard_logging_object fields and request tags async_log_success_event reads; B321 answers the tags
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SuccessEvent<'event> {
    pub call_type: Option<&'event str>,
    pub response_cost: Decimal,
    pub model_id: Option<&'event str>,
    pub custom_llm_provider: Option<&'event str>,
    pub tags: &'event [String],
}
