use rust_decimal::Decimal;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UsageTokens {
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    pub total_tokens: i64,
}

// The callback kwargs both adapters read: the response usage and the proxy's attribution fields.
#[derive(Clone, Copy, Debug)]
pub struct MeteringEvent<'event> {
    pub model: Option<&'event str>,
    pub response_cost: Option<Decimal>,
    pub usage: Option<UsageTokens>,
    pub response_id: Option<&'event str>,
    pub litellm_call_id: Option<&'event str>,
    pub request_user: Option<&'event str>,
    pub user_api_key_user_id: Option<&'event str>,
    pub user_api_key_team_id: Option<&'event str>,
}
