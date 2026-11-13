use super::database::DailyActivityDatabase;
use super::rows::adapters::export::export_rows;
use super::{
    DailyActivityRepository, DailyActivityRepositoryError, Result, USAGE_EXPORT_BATCH_SIZE,
};
use crate::daily_activity_contracts::{
    DailyActivityProxyReads, DailyActivityScope, ExportCursor, ExportRow, ExportType,
};
use crate::daily_activity_export::build_export_sql;
use chrono::{DateTime, Utc};

impl<Database, Reads, Clock> DailyActivityRepository<Database, Reads, Clock>
where
    Database: DailyActivityDatabase,
    Reads: DailyActivityProxyReads,
    Clock: Fn() -> DateTime<Utc>,
{
    pub fn export_rows<'repository>(
        &'repository self,
        scope: &'repository DailyActivityScope,
        export_type: ExportType,
    ) -> ExportRows<'repository, Database, Reads, Clock> {
        ExportRows {
            repository: self,
            scope,
            export_type,
            cursor: None,
            buffer: Vec::new().into_iter(),
            finished: false,
        }
    }
}

pub struct ExportRows<'repository, Database, Reads, Clock> {
    repository: &'repository DailyActivityRepository<Database, Reads, Clock>,
    scope: &'repository DailyActivityScope,
    export_type: ExportType,
    cursor: Option<ExportCursor>,
    buffer: std::vec::IntoIter<ExportRow>,
    finished: bool,
}

impl<Database, Reads, Clock> ExportRows<'_, Database, Reads, Clock>
where
    Database: DailyActivityDatabase,
    Reads: DailyActivityProxyReads,
    Clock: Fn() -> DateTime<Utc>,
{
    pub async fn next_row(&mut self) -> Result<Option<ExportRow>> {
        loop {
            if let Some(row) = self.buffer.next() {
                return Ok(Some(row));
            }
            if self.finished {
                return Ok(None);
            }
            let batch_size = i64::try_from(USAGE_EXPORT_BATCH_SIZE)
                .map_err(|_| DailyActivityRepositoryError::Pagination)?;
            let query = build_export_sql(
                self.scope,
                self.export_type,
                self.cursor.as_ref(),
                batch_size,
                &self.repository.clock,
            )?;
            let rows = self.repository.query(&query).await?;
            let batch = export_rows(&rows)?;
            if batch.len() == USAGE_EXPORT_BATCH_SIZE {
                self.cursor = next_cursor(&batch, self.export_type);
            } else {
                self.finished = true;
            }
            self.buffer = batch.into_iter();
        }
    }
}

#[must_use]
pub fn next_cursor(batch: &[ExportRow], export_type: ExportType) -> Option<ExportCursor> {
    let last = batch.last()?;
    let group_key = match export_type {
        ExportType::Daily => String::new(),
        ExportType::DailyWithKeys => last.api_key.clone().unwrap_or_default(),
        ExportType::DailyWithModels => last.model.clone().unwrap_or_default(),
        ExportType::DailyWithUsers => last.user_id.clone().unwrap_or_default(),
    };
    Some(ExportCursor {
        date: last.date.clone(),
        entity_id: last.entity_id.clone(),
        group_key,
    })
}
