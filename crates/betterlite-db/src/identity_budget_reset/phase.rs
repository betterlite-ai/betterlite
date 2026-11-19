#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdentityPhase {
    Keys,
    Users,
}

impl IdentityPhase {
    #[must_use]
    pub const fn call_type(self) -> &'static str {
        match self {
            Self::Keys => "reset_budget_keys",
            Self::Users => "reset_budget_users",
        }
    }

    #[must_use]
    pub const fn subject(self) -> &'static str {
        match self {
            Self::Keys => "keys",
            Self::Users => "users",
        }
    }

    #[must_use]
    pub const fn row_subject(self) -> &'static str {
        match self {
            Self::Keys => "key",
            Self::Users => "user",
        }
    }

    #[must_use]
    pub const fn found_count_key(self) -> &'static str {
        match self {
            Self::Keys => "num_keys_found",
            Self::Users => "num_users_found",
        }
    }

    #[must_use]
    pub const fn updated_count_key(self) -> &'static str {
        match self {
            Self::Keys => "num_keys_updated",
            Self::Users => "num_users_updated",
        }
    }

    #[must_use]
    pub const fn failed_count_key(self) -> &'static str {
        match self {
            Self::Keys => "num_keys_failed",
            Self::Users => "num_users_failed",
        }
    }

    #[must_use]
    pub const fn read_reason(self) -> &'static str {
        match self {
            Self::Keys => "reset_budget_read_keys_failure",
            Self::Users => "reset_budget_read_users_failure",
        }
    }

    #[must_use]
    pub const fn write_reason(self) -> &'static str {
        match self {
            Self::Keys => "reset_budget_write_keys_failure",
            Self::Users => "reset_budget_write_users_failure",
        }
    }
}
