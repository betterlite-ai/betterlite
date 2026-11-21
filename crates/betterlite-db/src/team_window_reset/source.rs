#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowSource {
    Keys,
    Teams,
}

impl WindowSource {
    // _WINDOW_SOURCES: the two tables whose rows carry per-window budget limits, swept in this order.
    pub const ALL: [Self; 2] = [Self::Keys, Self::Teams];

    #[must_use]
    pub const fn table(self) -> &'static str {
        match self {
            Self::Keys => "LiteLLM_VerificationToken",
            Self::Teams => "LiteLLM_TeamTable",
        }
    }

    #[must_use]
    pub const fn id_column(self) -> &'static str {
        match self {
            Self::Keys => "token",
            Self::Teams => "team_id",
        }
    }

    // Litellm_EntityType.value: the entity_type a window spend row is keyed by.
    #[must_use]
    pub const fn entity_type(self) -> &'static str {
        match self {
            Self::Keys => "key",
            Self::Teams => "team",
        }
    }

    #[must_use]
    pub const fn counter_prefix(self) -> &'static str {
        match self {
            Self::Keys => "spend:key",
            Self::Teams => "spend:team",
        }
    }

    #[must_use]
    pub const fn log_subject(self) -> &'static str {
        match self {
            Self::Keys => "keys",
            Self::Teams => "teams",
        }
    }

    #[must_use]
    pub const fn read_reason(self) -> &'static str {
        match self {
            Self::Keys => "reset_budget_read_key_windows_failure",
            Self::Teams => "reset_budget_read_team_windows_failure",
        }
    }

    #[must_use]
    pub const fn write_reason(self) -> &'static str {
        match self {
            Self::Keys => "reset_budget_write_key_windows_failure",
            Self::Teams => "reset_budget_write_team_windows_failure",
        }
    }

    // One keyset page ordered by the primary key, so the cursor strictly advances; the table and column names are module constants, never input.
    #[must_use]
    pub fn page_query(self) -> String {
        let table = self.table();
        let id_column = self.id_column();
        format!(
            "SELECT {id_column} AS row_id, budget_limits::text AS budget_limits FROM \"{table}\" WHERE budget_limits IS NOT NULL AND {id_column} > $1 ORDER BY {id_column} LIMIT $2"
        )
    }

    // The ORM's update of one row's Json? column: the payload is a serialized list stored as a JSON string, the way json.dumps lands in Prisma's Json field.
    #[must_use]
    pub fn update_query(self) -> String {
        let table = self.table();
        let id_column = self.id_column();
        format!("UPDATE \"{table}\" SET budget_limits = to_jsonb($2::text) WHERE {id_column} = $1")
    }

    // {counter_prefix}:{row_id}:window:{budget_duration}
    #[must_use]
    pub fn window_counter_key(self, row_id: &str, budget_duration: &str) -> String {
        let prefix = self.counter_prefix();
        format!("{prefix}:{row_id}:window:{budget_duration}")
    }
}

mod tests;
