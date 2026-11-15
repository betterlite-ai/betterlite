use url::Url;

pub const AZURE_OPENAI_V1_HOST_SUFFIXES: [&str; 2] =
    [".services.ai.azure.com", ".openai.azure.com"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApiKeyHeader {
    Authorization,
    ApiKey,
}

impl ApiKeyHeader {
    #[must_use]
    pub const fn wire_name(self) -> &'static str {
        match self {
            Self::Authorization => "Authorization",
            Self::ApiKey => "api-key",
        }
    }
}

#[must_use]
pub fn is_foundry_base(api_base: &str) -> bool {
    let Ok(parsed) = Url::parse(api_base) else {
        return false;
    };
    let Some(host) = parsed.host_str() else {
        return false;
    };
    if !host.ends_with(".services.ai.azure.com") {
        return false;
    }
    !parsed.path().contains("/openai/deployments")
}

#[must_use]
pub fn is_azure_openai_v1_host(api_base: Option<&str>) -> bool {
    api_base
        .and_then(|base| Url::parse(base).ok())
        .and_then(|parsed| parsed.host_str().map(str::to_owned))
        .is_some_and(|host| {
            AZURE_OPENAI_V1_HOST_SUFFIXES
                .iter()
                .any(|suffix| host.ends_with(suffix))
        })
}

#[must_use]
pub fn api_key_header_for_base(api_base: Option<&str>) -> ApiKeyHeader {
    if is_azure_openai_v1_host(api_base) {
        ApiKeyHeader::ApiKey
    } else {
        ApiKeyHeader::Authorization
    }
}
