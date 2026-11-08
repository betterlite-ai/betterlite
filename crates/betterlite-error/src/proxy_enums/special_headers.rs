use std::collections::BTreeSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpecialHeaders {
    Authorization,
    ApiKey,
    XApiKey,
    XGoogApiKey,
    OcpApimSubscriptionKey,
    XLitellmApiKey,
    McpAuth,
    McpServers,
    McpAccessGroups,
}

impl SpecialHeaders {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Authorization => "Authorization",
            Self::ApiKey => "API-Key",
            Self::XApiKey => "x-api-key",
            Self::XGoogApiKey => "x-goog-api-key",
            Self::OcpApimSubscriptionKey => "Ocp-Apim-Subscription-Key",
            Self::XLitellmApiKey => "x-litellm-api-key",
            Self::McpAuth => "x-mcp-auth",
            Self::McpServers => "x-mcp-servers",
            Self::McpAccessGroups => "x-mcp-access-groups",
        }
    }

    #[must_use]
    // ast-grep-ignore: no-long-names
    pub fn litellm_credential_header_names() -> BTreeSet<String> {
        [
            Self::Authorization,
            Self::ApiKey,
            Self::XApiKey,
            Self::XGoogApiKey,
            Self::OcpApimSubscriptionKey,
            Self::XLitellmApiKey,
        ]
        .map(Self::as_str)
        .map(str::to_lowercase)
        .into_iter()
        .collect()
    }
}
