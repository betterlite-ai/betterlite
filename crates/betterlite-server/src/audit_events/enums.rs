pub trait AuditName: Copy + PartialEq {
    const ENTRIES: &'static [(&'static str, Self)];

    #[must_use]
    fn as_str(self) -> &'static str {
        Self::ENTRIES
            .iter()
            .find_map(|(name, entry)| (*entry == self).then_some(*name))
            .unwrap_or_default()
    }

    #[must_use]
    fn parse(text: &str) -> Option<Self> {
        Self::ENTRIES
            .iter()
            .find_map(|(name, entry)| (*name == text).then_some(*entry))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuditAction {
    Created,
    Updated,
    Deleted,
    Blocked,
    Unblocked,
    Rotated,
    KillSwitchFired,
}

impl AuditName for AuditAction {
    const ENTRIES: &'static [(&'static str, Self)] = &[
        ("created", Self::Created),
        ("updated", Self::Updated),
        ("deleted", Self::Deleted),
        ("blocked", Self::Blocked),
        ("unblocked", Self::Unblocked),
        ("rotated", Self::Rotated),
        ("kill_switch_fired", Self::KillSwitchFired),
    ];
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuditTable {
    Team,
    User,
    Key,
    ProxyModel,
    ManagedFile,
    Tool,
    CacheConfig,
    ConfigOverrides,
    Config,
    SsoConfig,
    UiSettings,
    Agent,
}

impl AuditName for AuditTable {
    const ENTRIES: &'static [(&'static str, Self)] = &[
        ("LiteLLM_TeamTable", Self::Team),
        ("LiteLLM_UserTable", Self::User),
        ("LiteLLM_VerificationToken", Self::Key),
        ("LiteLLM_ProxyModelTable", Self::ProxyModel),
        ("LiteLLM_ManagedFileTable", Self::ManagedFile),
        ("LiteLLM_ToolTable", Self::Tool),
        ("LiteLLM_CacheConfig", Self::CacheConfig),
        ("LiteLLM_ConfigOverrides", Self::ConfigOverrides),
        ("LiteLLM_Config", Self::Config),
        ("LiteLLM_SSOConfig", Self::SsoConfig),
        ("LiteLLM_UISettings", Self::UiSettings),
        ("LiteLLM_AgentsTable", Self::Agent),
    ];
}
