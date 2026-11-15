use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AuditAction {
    #[serde(rename = "created")]
    Created,
    #[serde(rename = "updated")]
    Updated,
    #[serde(rename = "deleted")]
    Deleted,
    #[serde(rename = "blocked")]
    Blocked,
    #[serde(rename = "unblocked")]
    Unblocked,
    #[serde(rename = "rotated")]
    Rotated,
    #[serde(rename = "kill_switch_fired")]
    KillSwitchFired,
}

impl AuditAction {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Created => "created",
            Self::Updated => "updated",
            Self::Deleted => "deleted",
            Self::Blocked => "blocked",
            Self::Unblocked => "unblocked",
            Self::Rotated => "rotated",
            Self::KillSwitchFired => "kill_switch_fired",
        }
    }

    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        Some(match text {
            "created" => Self::Created,
            "updated" => Self::Updated,
            "deleted" => Self::Deleted,
            "blocked" => Self::Blocked,
            "unblocked" => Self::Unblocked,
            "rotated" => Self::Rotated,
            "kill_switch_fired" => Self::KillSwitchFired,
            _ => return None,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AuditTable {
    #[serde(rename = "LiteLLM_TeamTable")]
    Team,
    #[serde(rename = "LiteLLM_UserTable")]
    User,
    #[serde(rename = "LiteLLM_VerificationToken")]
    Key,
    #[serde(rename = "LiteLLM_ProxyModelTable")]
    ProxyModel,
    #[serde(rename = "LiteLLM_ManagedFileTable")]
    ManagedFile,
    #[serde(rename = "LiteLLM_ToolTable")]
    Tool,
    #[serde(rename = "LiteLLM_CacheConfig")]
    CacheConfig,
    #[serde(rename = "LiteLLM_ConfigOverrides")]
    ConfigOverrides,
    #[serde(rename = "LiteLLM_Config")]
    Config,
    #[serde(rename = "LiteLLM_SSOConfig")]
    SsoConfig,
    #[serde(rename = "LiteLLM_UISettings")]
    UiSettings,
    #[serde(rename = "LiteLLM_AgentsTable")]
    Agent,
}

impl AuditTable {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Team => "LiteLLM_TeamTable",
            Self::User => "LiteLLM_UserTable",
            Self::Key => "LiteLLM_VerificationToken",
            Self::ProxyModel => "LiteLLM_ProxyModelTable",
            Self::ManagedFile => "LiteLLM_ManagedFileTable",
            Self::Tool => "LiteLLM_ToolTable",
            Self::CacheConfig => "LiteLLM_CacheConfig",
            Self::ConfigOverrides => "LiteLLM_ConfigOverrides",
            Self::Config => "LiteLLM_Config",
            Self::SsoConfig => "LiteLLM_SSOConfig",
            Self::UiSettings => "LiteLLM_UISettings",
            Self::Agent => "LiteLLM_AgentsTable",
        }
    }

    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        Some(match text {
            "LiteLLM_TeamTable" => Self::Team,
            "LiteLLM_UserTable" => Self::User,
            "LiteLLM_VerificationToken" => Self::Key,
            "LiteLLM_ProxyModelTable" => Self::ProxyModel,
            "LiteLLM_ManagedFileTable" => Self::ManagedFile,
            "LiteLLM_ToolTable" => Self::Tool,
            "LiteLLM_CacheConfig" => Self::CacheConfig,
            "LiteLLM_ConfigOverrides" => Self::ConfigOverrides,
            "LiteLLM_Config" => Self::Config,
            "LiteLLM_SSOConfig" => Self::SsoConfig,
            "LiteLLM_UISettings" => Self::UiSettings,
            "LiteLLM_AgentsTable" => Self::Agent,
            _ => return None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{AuditAction, AuditTable};

    #[test]
    fn audit_action_values_round_trip() {
        for value in [
            "created",
            "updated",
            "deleted",
            "blocked",
            "unblocked",
            "rotated",
            "kill_switch_fired",
        ] {
            let action = AuditAction::parse(value).unwrap();
            assert_eq!(action.as_str(), value);
        }
        assert_eq!(AuditAction::parse("changed"), None);
    }

    #[test]
    fn audit_table_values_round_trip() {
        for value in [
            "LiteLLM_TeamTable",
            "LiteLLM_UserTable",
            "LiteLLM_VerificationToken",
            "LiteLLM_ProxyModelTable",
            "LiteLLM_ManagedFileTable",
            "LiteLLM_ToolTable",
            "LiteLLM_CacheConfig",
            "LiteLLM_ConfigOverrides",
            "LiteLLM_Config",
            "LiteLLM_SSOConfig",
            "LiteLLM_UISettings",
            "LiteLLM_AgentsTable",
        ] {
            let table = AuditTable::parse(value).unwrap();
            assert_eq!(table.as_str(), value);
        }
        assert_eq!(AuditTable::parse("LiteLLM_KeysTable"), None);
    }
}
