use polars::error::ErrString;
use std::collections::BTreeMap;

use super::schema::focus_normalized_schema;
use polars::prelude::{
    Column, DataFrame, DataType, IntoSeries as _, NamedFrom as _, PlSmallStr, PolarsError, Series,
    TimeUnit,
};

mod projection;

const TAG_KEYS: [&str; 10] = [
    "team_id",
    "team_alias",
    "organization_id",
    "organization_alias",
    "user_id",
    "user_email",
    "api_key_alias",
    "model",
    "model_group",
    "custom_llm_provider",
];

pub struct FocusTransformer;

impl FocusTransformer {
    pub fn transform(&self, mut frame: DataFrame) -> Result<DataFrame, PolarsError> {
        if frame.is_empty() {
            return Ok(DataFrame::empty_with_schema(&focus_normalized_schema()));
        }
        let tags = build_tags(&frame)?;
        frame.with_column(tags)?;
        let usage_date = frame
            .column("date")?
            .cast(&DataType::Date)?
            .cast(&DataType::Datetime(TimeUnit::Microseconds, None))?;
        let charge_end = next_day(&usage_date)?;
        frame.with_column(usage_date.with_name(PlSmallStr::from("ChargePeriodStart")))?;
        frame.with_column(charge_end)?;
        let columns = projection::COLUMNS
            .into_iter()
            .map(|(name, source)| {
                source
                    .project(&frame)
                    .map(|column| column.with_name(PlSmallStr::from(name)))
            })
            .collect::<Result<Vec<_>, _>>()?;
        DataFrame::new(columns)
    }
}

fn next_day(dates: &Column) -> Result<Column, PolarsError> {
    let values = dates
        .datetime()?
        .physical()
        .into_iter()
        .map(|value| {
            value
                .map(|micros| {
                    micros.checked_add(86_400_000_000_i64).ok_or_else(|| {
                        PolarsError::ComputeError(ErrString::from("FOCUS period overflow"))
                    })
                })
                .transpose()
        })
        .collect::<Result<Vec<_>, _>>()?;
    let series = Series::new(PlSmallStr::from("ChargePeriodEnd"), values);
    Ok(Column::from(series.cast(dates.dtype())?))
}

fn build_tags(frame: &DataFrame) -> Result<Series, PolarsError> {
    let mut values = Vec::with_capacity(frame.height());
    for row in 0..frame.height() {
        let mut tags = BTreeMap::new();
        for key in TAG_KEYS {
            let Ok(column) = frame.column(key) else {
                continue;
            };
            if let Some(value) = column
                .get(row)
                .ok()
                .and_then(|value| value.extract_str().map(str::to_owned))
            {
                tags.insert(key, value);
            }
        }
        values.push(
            serde_json::to_string(&tags)
                .map_err(|error| PolarsError::ComputeError(ErrString::from(error.to_string())))?,
        );
    }
    Ok(Series::new(PlSmallStr::from("Tags"), values))
}

fn decimal(frame: &DataFrame, source: &str) -> Result<Column, PolarsError> {
    let values = frame.column(source)?.cast(&DataType::Float64)?;
    values.cast(&DataType::Decimal(Some(18), Some(6)))
}

fn formatted_datetime(frame: &DataFrame, source: &str) -> Result<Column, PolarsError> {
    let values = frame
        .column(source)?
        .datetime()?
        .strftime("%Y-%m-%dT%H:%M:%SZ")?;
    Ok(Column::from(values.into_series()))
}
