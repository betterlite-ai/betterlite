const TABLE: &str = "\"LiteLLM_DeletedVerificationToken\"";

#[derive(Clone, Copy)]
pub(super) enum ColumnKind {
    Text,
    Boolean,
    Integer,
    Number,
    Timestamp,
    Json,
    TextArray,
}

impl ColumnKind {
    pub(super) const fn cast(self) -> &'static str {
        match self {
            Self::Text | Self::Boolean | Self::Timestamp | Self::Json | Self::TextArray => "",
            Self::Integer => "::bigint",
            Self::Number => "::numeric::double precision",
        }
    }
}

pub(super) struct Column {
    name: &'static str,
    kind: ColumnKind,
}

pub(super) static COLUMNS: [Column; 51] = [
    Column {
        name: "id",
        kind: ColumnKind::Text,
    },
    Column {
        name: "token",
        kind: ColumnKind::Text,
    },
    Column {
        name: "key_name",
        kind: ColumnKind::Text,
    },
    Column {
        name: "key_alias",
        kind: ColumnKind::Text,
    },
    Column {
        name: "user_id",
        kind: ColumnKind::Text,
    },
    Column {
        name: "team_id",
        kind: ColumnKind::Text,
    },
    Column {
        name: "agent_id",
        kind: ColumnKind::Text,
    },
    Column {
        name: "project_id",
        kind: ColumnKind::Text,
    },
    Column {
        name: "budget_duration",
        kind: ColumnKind::Text,
    },
    Column {
        name: "key_type",
        kind: ColumnKind::Text,
    },
    Column {
        name: "budget_id",
        kind: ColumnKind::Text,
    },
    Column {
        name: "organization_id",
        kind: ColumnKind::Text,
    },
    Column {
        name: "object_permission_id",
        kind: ColumnKind::Text,
    },
    Column {
        name: "created_by",
        kind: ColumnKind::Text,
    },
    Column {
        name: "updated_by",
        kind: ColumnKind::Text,
    },
    Column {
        name: "rotation_interval",
        kind: ColumnKind::Text,
    },
    Column {
        name: "deleted_by",
        kind: ColumnKind::Text,
    },
    Column {
        name: "deleted_by_api_key",
        kind: ColumnKind::Text,
    },
    Column {
        name: "litellm_changed_by",
        kind: ColumnKind::Text,
    },
    Column {
        name: "soft_budget_cooldown",
        kind: ColumnKind::Boolean,
    },
    Column {
        name: "blocked",
        kind: ColumnKind::Boolean,
    },
    Column {
        name: "auto_rotate",
        kind: ColumnKind::Boolean,
    },
    Column {
        name: "max_parallel_requests",
        kind: ColumnKind::Integer,
    },
    Column {
        name: "tpm_limit",
        kind: ColumnKind::Integer,
    },
    Column {
        name: "rpm_limit",
        kind: ColumnKind::Integer,
    },
    Column {
        name: "tpd_limit",
        kind: ColumnKind::Integer,
    },
    Column {
        name: "rotation_count",
        kind: ColumnKind::Integer,
    },
    Column {
        name: "spend",
        kind: ColumnKind::Number,
    },
    Column {
        name: "total_spend",
        kind: ColumnKind::Number,
    },
    Column {
        name: "max_budget",
        kind: ColumnKind::Number,
    },
    Column {
        name: "expires",
        kind: ColumnKind::Timestamp,
    },
    Column {
        name: "budget_reset_at",
        kind: ColumnKind::Timestamp,
    },
    Column {
        name: "created_at",
        kind: ColumnKind::Timestamp,
    },
    Column {
        name: "updated_at",
        kind: ColumnKind::Timestamp,
    },
    Column {
        name: "settings_updated_at",
        kind: ColumnKind::Timestamp,
    },
    Column {
        name: "last_active",
        kind: ColumnKind::Timestamp,
    },
    Column {
        name: "last_rotation_at",
        kind: ColumnKind::Timestamp,
    },
    Column {
        name: "key_rotation_at",
        kind: ColumnKind::Timestamp,
    },
    Column {
        name: "deleted_at",
        kind: ColumnKind::Timestamp,
    },
    Column {
        name: "models",
        kind: ColumnKind::TextArray,
    },
    Column {
        name: "allowed_cache_controls",
        kind: ColumnKind::TextArray,
    },
    Column {
        name: "allowed_routes",
        kind: ColumnKind::TextArray,
    },
    Column {
        name: "access_group_ids",
        kind: ColumnKind::TextArray,
    },
    Column {
        name: "aliases",
        kind: ColumnKind::Json,
    },
    Column {
        name: "config",
        kind: ColumnKind::Json,
    },
    Column {
        name: "permissions",
        kind: ColumnKind::Json,
    },
    Column {
        name: "metadata",
        kind: ColumnKind::Json,
    },
    Column {
        name: "model_spend",
        kind: ColumnKind::Json,
    },
    Column {
        name: "model_max_budget",
        kind: ColumnKind::Json,
    },
    Column {
        name: "budget_fallbacks",
        kind: ColumnKind::Json,
    },
    Column {
        name: "router_settings",
        kind: ColumnKind::Json,
    },
];

pub(super) fn insert_statement(rows: usize) -> String {
    let columns = COLUMNS
        .iter()
        .map(|column| format!("\"{}\"", column.name))
        .collect::<Vec<String>>()
        .join(", ");
    let row_width = COLUMNS.len();
    let tuples = (0..rows)
        .map(|row| {
            let placeholders = COLUMNS
                .iter()
                .enumerate()
                .map(|(index, column)| {
                    let offset = row.saturating_mul(row_width).saturating_add(index);
                    let position = offset.saturating_add(1);
                    format!("${position}{}", column.kind.cast())
                })
                .collect::<Vec<String>>()
                .join(", ");
            format!("({placeholders})")
        })
        .collect::<Vec<String>>()
        .join(", ");
    format!("INSERT INTO {TABLE} ({columns}) VALUES {tuples}")
}
