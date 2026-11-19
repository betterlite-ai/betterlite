use axum::response::{IntoResponse, Response};
use chrono::{DateTime, Utc};

use super::dates::{DateFormatError, DateFormats, parse_date};
use super::filters::{EqualityColumn, EqualityFilter, MetadataFilter, SpendBounds};
use super::validate::{proxy_bad_request, proxy_converted_http_error};
use super::{DateWindowQuery, ScalarFilters, SpendLogQuery};

const MISSING_DATES_MESSAGE: &str = "Start date and end date are required";
const NONE_PARAMETER: &str = "None";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpendLogRoute {
    Ui,
    PublicV2,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpendLogWindow {
    pub start_inclusive: DateTime<Utc>,
    pub end_inclusive: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpendLogSearch {
    pub value: String,
    pub window: SpendLogWindow,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedSpendLogQuery {
    pub window: Option<SpendLogWindow>,
    pub search_owns_window: bool,
    pub search: Option<SpendLogSearch>,
    pub request_id: Option<String>,
    pub equality: Vec<EqualityFilter>,
    pub metadata: Vec<MetadataFilter>,
    pub spend: SpendBounds,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PrepareError {
    #[error("Start date and end date are required")]
    MissingDates,
    #[error(transparent)]
    InvalidDate {
        #[from]
        source: DateFormatError,
    },
}

impl IntoResponse for PrepareError {
    fn into_response(self) -> Response {
        match self {
            Self::MissingDates => {
                proxy_bad_request(MISSING_DATES_MESSAGE.to_owned(), NONE_PARAMETER)
            }
            Self::InvalidDate { source } => proxy_converted_http_error(source.to_string()),
        }
    }
}

impl SpendLogQuery {
    pub fn prepare(&self, route: SpendLogRoute) -> Result<PreparedSpendLogQuery, PrepareError> {
        let is_v2 = matches!(route, SpendLogRoute::PublicV2);
        let is_request_id_lookup = self.filters.request_id.is_some() && !is_v2;
        let is_search_lookup = self.filters.search.is_some();
        let search_owns_window = is_search_lookup && !is_v2;
        let window = if is_request_id_lookup && !is_search_lookup {
            None
        } else {
            Some(parse_window(&self.dates, route)?)
        };
        let search = self
            .filters
            .search
            .as_ref()
            .zip(window.as_ref())
            .map(|(term, bounds)| SpendLogSearch {
                value: term.clone(),
                window: bounds.clone(),
            });
        Ok(PreparedSpendLogQuery {
            window,
            search_owns_window,
            search,
            request_id: self.filters.request_id.clone(),
            equality: equality_filters(&self.filters),
            metadata: metadata_filters(&self.filters),
            spend: self.spend.clone(),
        })
    }
}

fn parse_window(
    dates: &DateWindowQuery,
    route: SpendLogRoute,
) -> Result<SpendLogWindow, PrepareError> {
    let formats = match route {
        SpendLogRoute::Ui => DateFormats::SecondsOnly,
        SpendLogRoute::PublicV2 => DateFormats::SecondsOrDateOnly,
    };
    let (Some(start_date), Some(end_date)) = (&dates.start_date, &dates.end_date) else {
        return Err(PrepareError::MissingDates);
    };
    Ok(SpendLogWindow {
        start_inclusive: parse_date(start_date, formats)?,
        end_inclusive: parse_date(end_date, formats)?,
    })
}

fn equality_filters(filters: &ScalarFilters) -> Vec<EqualityFilter> {
    EqualityColumn::WHERE_CONDITION_ORDER
        .into_iter()
        .filter_map(|column| {
            column.value_in(filters).map(|value| EqualityFilter {
                column,
                value: value.to_owned(),
            })
        })
        .collect()
}

fn metadata_filters(filters: &ScalarFilters) -> Vec<MetadataFilter> {
    let mut metadata = Vec::new();
    if let Some(value) = &filters.key_alias {
        metadata.push(MetadataFilter::KeyAlias(value.clone()));
    }
    if let Some(value) = &filters.error_code {
        metadata.push(MetadataFilter::ErrorCode(value.clone()));
    }
    if let Some(value) = &filters.error_message {
        metadata.push(MetadataFilter::ErrorMessage(value.clone()));
    }
    metadata
}
