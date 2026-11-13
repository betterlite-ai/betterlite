#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncedTable {
    ProxyModelTable,
    CredentialsTable,
    GuardrailsTable,
    PolicyTable,
    PolicyAttachmentTable,
    ManagedVectorStoresTable,
    ManagedVectorStoreIndexTable,
    McpServerTable,
    AgentsTable,
    PromptTable,
    SearchToolsTable,
    SsoConfig,
    CacheConfig,
    ConfigOverrides,
    UiSettings,
}

impl SyncedTable {
    pub const ALL: [Self; 15] = [
        Self::ProxyModelTable,
        Self::CredentialsTable,
        Self::GuardrailsTable,
        Self::PolicyTable,
        Self::PolicyAttachmentTable,
        Self::ManagedVectorStoresTable,
        Self::ManagedVectorStoreIndexTable,
        Self::McpServerTable,
        Self::AgentsTable,
        Self::PromptTable,
        Self::SearchToolsTable,
        Self::SsoConfig,
        Self::CacheConfig,
        Self::ConfigOverrides,
        Self::UiSettings,
    ];

    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::ProxyModelTable => "litellm_proxymodeltable",
            Self::CredentialsTable => "litellm_credentialstable",
            Self::GuardrailsTable => "litellm_guardrailstable",
            Self::PolicyTable => "litellm_policytable",
            Self::PolicyAttachmentTable => "litellm_policyattachmenttable",
            Self::ManagedVectorStoresTable => "litellm_managedvectorstorestable",
            Self::ManagedVectorStoreIndexTable => "litellm_managedvectorstoreindextable",
            Self::McpServerTable => "litellm_mcpservertable",
            Self::AgentsTable => "litellm_agentstable",
            Self::PromptTable => "litellm_prompttable",
            Self::SearchToolsTable => "litellm_searchtoolstable",
            Self::SsoConfig => "litellm_ssoconfig",
            Self::CacheConfig => "litellm_cacheconfig",
            Self::ConfigOverrides => "litellm_configoverrides",
            Self::UiSettings => "litellm_uisettings",
        }
    }
}
