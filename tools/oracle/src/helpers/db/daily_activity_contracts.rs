use serde_json::Value;

pub(crate) struct Op;

macro_rules! contract {
    ($value:expr, $type:path) => {
        serde_json::to_value(Op::parse::<$type>($value)?)
            .map_err(|error| Op::error(error.to_string()))
    };
}

impl crate::helpers::Helper for Op {
    type Input = Value;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let (kind, value) = Self::inputs(input)?;
        match kind.as_str() {
            "scope" => Self::scope(value),
            "key_spend" => contract!(value, betterlite_db::daily_activity_contracts::KeySpendRow),
            "key_page" => contract!(value, betterlite_db::daily_activity_contracts::KeyPage),
            "key_metadata" => contract!(
                value,
                betterlite_db::daily_activity_contracts::KeyMetadataRow
            ),
            "export" => contract!(value, betterlite_db::daily_activity_contracts::ExportRow),
            "rollup" => contract!(
                value,
                betterlite_db::daily_activity_contracts::RollupMetricsRow
            ),
            "grouping" => contract!(
                value,
                betterlite_db::daily_activity_contracts::GroupingSetsRow
            ),
            "entity" => contract!(
                value,
                betterlite_db::daily_activity_contracts::EntityRollupRow
            ),
            "aggregated" => contract!(
                value,
                betterlite_db::daily_activity_contracts::AggregatedRows
            ),
            "daily_page" => contract!(
                value,
                betterlite_db::daily_activity_contracts::DailyRowsPage
            ),
            _ => Err(Box::new(crate::helpers::HelperError {
                class: "ValueError".to_owned(),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message: format!("unknown contract {kind:?}"),
            })),
        }
    }
}

impl Op {
    fn inputs(input: &Value) -> Result<(String, Value), Box<crate::helpers::HelperError>> {
        let step = input
            .get("step")
            .ok_or_else(|| Self::error("step must be an object".to_owned()))?;
        let kind = step
            .get("kind")
            .and_then(Value::as_str)
            .ok_or_else(|| Self::error("unknown contract".to_owned()))?
            .to_owned();
        let value = step
            .get("value")
            .cloned()
            .ok_or_else(|| Self::error("value is required".to_owned()))?;
        Ok((kind, value))
    }

    fn scope(value: Value) -> crate::helpers::HelperResult {
        let scope =
            Self::parse::<betterlite_db::daily_activity_contracts::DailyActivityScope>(value)?;
        let validated =
            betterlite_db::daily_activity_contracts::validate_scope(scope).map_err(Self::error)?;
        serde_json::to_value(validated).map_err(|error| Self::error(error.to_string()))
    }

    fn parse<T>(value: Value) -> Result<T, Box<crate::helpers::HelperError>>
    where
        T: serde::de::DeserializeOwned,
    {
        match serde_json::from_value(value) {
            Ok(result) => Ok(result),
            Err(error) => Err(Self::error(error.to_string())),
        }
    }

    fn error(message: String) -> Box<crate::helpers::HelperError> {
        let error_message = message;
        Box::new(crate::helpers::HelperError {
            class: "ValueError".to_owned(),
            status: None,
            error_type: None,
            param: None,
            code: None,
            message: error_message,
        })
    }
}
