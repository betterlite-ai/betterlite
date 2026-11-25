use rust_decimal::Decimal;

// constants.LITELLM_PROXY_BUDGET_NAME: the user row a positive global max_budget charges.
pub const DEFAULT_PROXY_BUDGET_NAME: &str = "litellm-proxy-budget";

// What the writer reads once at startup: B1714's switch, the proxy budget identity and its threshold.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpendWriterSettings {
    pub disable_spend_updates: bool,
    // None stands for a present null, which is neither the True nor the False the source compares against.
    pub disable_spend_logs: Option<bool>,
    pub max_budget: Decimal,
    pub proxy_budget_name: String,
}

impl Default for SpendWriterSettings {
    fn default() -> Self {
        Self {
            disable_spend_updates: false,
            disable_spend_logs: Some(false),
            max_budget: Decimal::ZERO,
            proxy_budget_name: DEFAULT_PROXY_BUDGET_NAME.to_owned(),
        }
    }
}
