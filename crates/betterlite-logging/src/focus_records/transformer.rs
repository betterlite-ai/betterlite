use std::collections::BTreeMap;

use polars::prelude::{
    Column, DataFrame, DataType, IntoSeries, NamedFrom, PolarsError, Series, TimeUnit,
};

use super::schema::focus_normalized_schema;

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
    pub fn transform(&self, frame: DataFrame) -> PolarsResult<DataFrame> {
        if frame.is_empty() {
            return Ok(DataFrame::empty_with_schema(&focus_normalized_schema()));
        }
        let mut frame = frame;
        let tags = build_tags(&frame)?;
        frame.with_column(tags)?;
        let usage_date = frame
            .column("date")?
            .cast(&DataType::Date)?
            .cast(&DataType::Datetime(TimeUnit::Microseconds, None))?;
        let charge_end = &usage_date + 86_400_000_000_i64;
        let charge_start = usage_date.clone().with_name("ChargePeriodStart".into());
        let charge_end = charge_end.with_name("ChargePeriodEnd".into());
        frame.with_column(charge_start)?;
        frame.with_column(charge_end)?;
        let mut output = DataFrame::new(vec![
            decimal(&frame, "spend", "BilledCost")?,
            string_column(&frame, "api_key", "BillingAccountId")?,
            string_column(&frame, "api_key_alias", "BillingAccountName")?,
            constant_string("BillingAccountType", "API Key", frame.height()),
            constant_string("BillingCurrency", "USD", frame.height()),
            formatted_datetime(&frame, "ChargePeriodEnd", "BillingPeriodEnd")?,
            formatted_datetime(&frame, "ChargePeriodStart", "BillingPeriodStart")?,
            constant_string("ChargeCategory", "Usage", frame.height()),
            null_strings("ChargeClass", frame.height())?,
            string_column(&frame, "model", "ChargeDescription")?,
            constant_string("ChargeFrequency", "Usage-Based", frame.height()),
            formatted_datetime(&frame, "ChargePeriodEnd", "ChargePeriodEndText")?,
            formatted_datetime(&frame, "ChargePeriodStart", "ChargePeriodStartText")?,
            decimal(&frame, "api_requests", "ConsumedQuantity")?,
            constant_string("ConsumedUnit", "Requests", frame.height()),
            decimal(&frame, "spend", "ContractedCost")?,
            null_strings("ContractedUnitPrice", frame.height())?,
            decimal(&frame, "spend", "EffectiveCost")?,
            string_column(&frame, "custom_llm_provider", "InvoiceIssuerName")?,
            null_strings("InvoiceId", frame.height())?,
            decimal(&frame, "spend", "ListCost")?,
            null_decimals("ListUnitPrice", frame.height())?,
            null_strings("AvailabilityZone", frame.height())?,
            constant_string("PricingCurrency", "USD", frame.height()),
            null_strings("PricingCategory", frame.height())?,
            decimal(&frame, "api_requests", "PricingQuantity")?,
            null_decimals("PricingCurrencyContractedUnitPrice", frame.height())?,
            decimal(&frame, "spend", "PricingCurrencyEffectiveCost")?,
            null_decimals("PricingCurrencyListUnitPrice", frame.height())?,
            constant_string("PricingUnit", "Requests", frame.height()),
            string_column(&frame, "custom_llm_provider", "ProviderName")?,
            string_column(&frame, "custom_llm_provider", "PublisherName")?,
            null_strings("RegionId", frame.height())?,
            null_strings("RegionName", frame.height())?,
            string_column(&frame, "model", "ResourceId")?,
            string_column(&frame, "model", "ResourceName")?,
            string_column(&frame, "model", "ResourceType")?,
            constant_string("ServiceCategory", "AI and Machine Learning", frame.height()),
            constant_string("ServiceSubcategory", "Generative AI", frame.height()),
            string_column(&frame, "model_group", "ServiceName")?,
            string_column(&frame, "team_id", "SubAccountId")?,
            string_column(&frame, "team_alias", "SubAccountName")?,
            null_strings("SubAccountType", frame.height())?,
            string_column(&frame, "Tags", "Tags")?,
        ])?;
        output.rename("ChargePeriodStartText", "ChargePeriodStart".into())?;
        output.rename("ChargePeriodEndText", "ChargePeriodEnd".into())?;
        Ok(output)
    }
}

fn build_tags(frame: &DataFrame) -> PolarsResult<Series> {
    let mut values = Vec::with_capacity(frame.height());
    for row in 0..frame.height() {
        let mut tags = BTreeMap::new();
        for key in TAG_KEYS {
            if let Ok(column) = frame.column(key) {
                if let Some(value) = column
                    .get(row)
                    .ok()
                    .and_then(|value| value.extract_str().map(str::to_owned))
                {
                    tags.insert(key, value);
                }
            }
        }
        values.push(
            serde_json::to_string(&tags)
                .map_err(|error| PolarsError::ComputeError(error.to_string().into()))?,
        );
    }
    Ok(Series::new("Tags".into(), values))
}

fn decimal(frame: &DataFrame, source: &str, name: &str) -> PolarsResult<Column> {
    let values = frame.column(source)?.cast(&DataType::Float64)?;
    let values = values.cast(&DataType::Decimal(Some(18), Some(6)))?;
    Ok(values.with_name(name.into()).clone())
}

fn string_column(frame: &DataFrame, source: &str, name: &str) -> PolarsResult<Column> {
    Ok(frame
        .column(source)?
        .cast(&DataType::String)?
        .with_name(name.into())
        .clone())
}

fn constant_string(name: &str, value: &str, height: usize) -> Column {
    Column::new(name.into(), vec![value; height])
}

fn null_strings(name: &str, height: usize) -> PolarsResult<Column> {
    Ok(Column::new(name.into(), vec![None::<&str>; height]))
}

fn null_decimals(name: &str, height: usize) -> PolarsResult<Column> {
    Ok(Column::new(name.into(), vec![None::<f64>; height])
        .cast(&DataType::Decimal(Some(18), Some(6)))?)
}

fn formatted_datetime(frame: &DataFrame, source: &str, name: &str) -> PolarsResult<Column> {
    let values = frame
        .column(source)?
        .datetime()?
        .strftime("%Y-%m-%dT%H:%M:%SZ")?;
    Ok(values.into_series().with_name(name.into()).clone().into())
}

type PolarsResult<T> = Result<T, PolarsError>;
