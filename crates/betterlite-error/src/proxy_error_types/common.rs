#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommonProxyErrors {
    DbNotConnectedError,
    NoLlmRouter,
    NotAllowedAccess,
    NotPremiumUser,
    MaxParallelRequestLimitReached,
    MissingEnterprisePackage,
    MissingEnterprisePackageDocker,
}

impl CommonProxyErrors {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::DbNotConnectedError => {
                "DB not connected. This endpoint needs a database; set DATABASE_URL to a PostgreSQL connection string (postgresql://...) to enable it. See https://docs.litellm.ai/docs/proxy/virtual_keys"
            }
            Self::NoLlmRouter => "No models configured on proxy",
            Self::NotAllowedAccess => "Admin-only endpoint. Not allowed to access this.",
            Self::NotPremiumUser => {
                "You must be a LiteLLM Enterprise user to use this feature. If you have a license please set `LITELLM_LICENSE` in your env. Get a 7 day trial key here: https://www.litellm.ai/enterprise#trial. \nPricing: https://www.litellm.ai/#pricing"
            }
            Self::MaxParallelRequestLimitReached => {
                "Crossed TPM / RPM / Max Parallel Request Limit"
            }
            Self::MissingEnterprisePackage => {
                "Missing litellm-enterprise package. Please install it to use this feature. Run `pip install litellm-enterprise`"
            }
            Self::MissingEnterprisePackageDocker => {
                "This uses the enterprise folder - only available on the Docker image."
            }
        }
    }
}
