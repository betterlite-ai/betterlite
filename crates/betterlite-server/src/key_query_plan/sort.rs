#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortColumn {
    Spend,
    MaxBudget,
    CreatedAt,
    UpdatedAt,
    Token,
    KeyAlias,
}

// the order LiteLLM both validates against and names in its 400 detail
pub const SORT_COLUMNS: [SortColumn; 6] = [
    SortColumn::Spend,
    SortColumn::MaxBudget,
    SortColumn::CreatedAt,
    SortColumn::UpdatedAt,
    SortColumn::Token,
    SortColumn::KeyAlias,
];

impl SortColumn {
    #[must_use]
    pub const fn from_wire(text: &str) -> Option<Self> {
        match text.as_bytes() {
            b"spend" => Some(Self::Spend),
            b"max_budget" => Some(Self::MaxBudget),
            b"created_at" => Some(Self::CreatedAt),
            b"updated_at" => Some(Self::UpdatedAt),
            b"token" => Some(Self::Token),
            b"key_alias" => Some(Self::KeyAlias),
            _ => None,
        }
    }

    #[must_use]
    pub const fn wire(self) -> &'static str {
        match self {
            Self::Spend => "spend",
            Self::MaxBudget => "max_budget",
            Self::CreatedAt => "created_at",
            Self::UpdatedAt => "updated_at",
            Self::Token => "token",
            Self::KeyAlias => "key_alias",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum SortOrder {
    #[default]
    Asc,
    Desc,
}

impl SortOrder {
    #[must_use]
    pub const fn from_wire(text: &str) -> Option<Self> {
        match text.as_bytes() {
            b"asc" => Some(Self::Asc),
            b"desc" => Some(Self::Desc),
            _ => None,
        }
    }

    #[must_use]
    pub const fn sql(self) -> &'static str {
        match self {
            Self::Asc => "ASC",
            Self::Desc => "DESC",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SortBy {
    pub column: SortColumn,
    pub order: SortOrder,
}

// LiteLLM falls back to these two keys only when the caller named no sort column
pub const DEFAULT_SORT: [SortBy; 2] = [
    SortBy {
        column: SortColumn::CreatedAt,
        order: SortOrder::Desc,
    },
    SortBy {
        column: SortColumn::Token,
        order: SortOrder::Desc,
    },
];

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SortError {
    #[error("Invalid sort column. Must be one of: {columns}")]
    Column { columns: String },
    #[error("Invalid sort order. Must be 'asc' or 'desc'")]
    Order,
}

impl SortError {
    // LiteLLM raises HTTPException(status_code=400) for both the column and the order
    pub const STATUS_CODE: u16 = 400;
}

pub fn validate_sort_params(
    sort_by: Option<&str>,
    sort_order: &str,
) -> Result<Option<SortBy>, SortError> {
    match sort_by {
        None => Ok(None),
        Some(column) => validate_sort_column(column, sort_order).map(Some),
    }
}

pub fn validate_sort_column(sort_by: &str, sort_order: &str) -> Result<SortBy, SortError> {
    let Some(column) = SortColumn::from_wire(sort_by) else {
        return Err(SortError::Column {
            columns: permitted_columns(),
        });
    };
    let lowered = sort_order.to_lowercase();
    let Some(order) = SortOrder::from_wire(&lowered) else {
        return Err(SortError::Order);
    };
    Ok(SortBy { column, order })
}

fn permitted_columns() -> String {
    SORT_COLUMNS
        .iter()
        .map(|column| column.wire())
        .collect::<Vec<_>>()
        .join(", ")
}
