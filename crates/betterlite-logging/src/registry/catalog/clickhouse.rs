mod adapter;
mod payload;
mod rows;
mod schema;
mod storage;
mod tests;

use std::sync::Arc;

use super::{CatalogError, CatalogInputs};
use crate::callback_logger::CallbackLogger;
use crate::clickhouse::clickhouse_logging::batch_logger::ClickHouseBatchLogger;
use crate::clickhouse::clickhouse_logging::context::LensAnalysis;
use crate::clickhouse::clickhouse_logging::schema::SPEND_LOGS_TABLE;
use crate::clickhouse::clickhouse_logging::settings::ClickHouseSettings;
use crate::clickhouse::clickhouse_logging::spend_logger::ClickHouseSpendLogger;
use crate::finops::metering::clock::SystemClock;
use adapter::ClickHouseAdapter;
use storage::ClickHouseHttpStorage;

pub fn build(inputs: &CatalogInputs) -> Result<Arc<dyn CallbackLogger>, CatalogError> {
    let settings = ClickHouseSettings::from_environment(inputs.env.as_ref()).map_err(|error| {
        CatalogError::Init {
            message: error.to_string(),
        }
    })?;
    let storage = ClickHouseHttpStorage::new(
        inputs.http.clone(),
        &settings.database,
        &settings.url,
        SystemClock,
    )
    .map_err(|error| CatalogError::Init {
        message: error.to_string(),
    })?;
    let batch =
        ClickHouseBatchLogger::new(storage, SPEND_LOGS_TABLE, settings, inputs.tracker.clone());
    Ok(Arc::new(ClickHouseAdapter {
        logger: ClickHouseSpendLogger { batch },
        lens: LensAnalysis::default(),
    }))
}
