macro_rules! audit_names {
    ($name:ident { $($variant:ident => $text:literal),+ $(,)? }) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub enum $name {
            $($variant),+
        }

        impl $name {
            #[must_use]
            pub const fn as_str(self) -> &'static str {
                match self {
                    $(Self::$variant => $text),+
                }
            }

            #[must_use]
            pub fn parse(text: &str) -> Option<Self> {
                match text {
                    $($text => Some(Self::$variant),)+
                    _ => None,
                }
            }
        }
    };
}

audit_names! {
    AuditAction {
        Created => "created",
        Updated => "updated",
        Deleted => "deleted",
        Blocked => "blocked",
        Unblocked => "unblocked",
        Rotated => "rotated",
        KillSwitchFired => "kill_switch_fired",
    }
}

audit_names! {
    AuditTable {
        Team => "LiteLLM_TeamTable",
        User => "LiteLLM_UserTable",
        Key => "LiteLLM_VerificationToken",
        ProxyModel => "LiteLLM_ProxyModelTable",
        ManagedFile => "LiteLLM_ManagedFileTable",
        Tool => "LiteLLM_ToolTable",
        CacheConfig => "LiteLLM_CacheConfig",
        ConfigOverrides => "LiteLLM_ConfigOverrides",
        Config => "LiteLLM_Config",
        SsoConfig => "LiteLLM_SSOConfig",
        UiSettings => "LiteLLM_UISettings",
        Agent => "LiteLLM_AgentsTable",
    }
}
