use std::error::Error as StdError;
use std::future::Future;

use betterlite_db::budget_permission_rows::JsonData;
use betterlite_roi::records::{ROIEstimateStatus, ROIPullRecord, ROIReport};

use super::error::CacheWriteError;

pub const REPORT_PARAM: &str = "roi_calculator_report";
pub const PULL_PARAM_PREFIX: &str = "roi_calculator_pull_";

pub trait ReportRepository {
    type Error: StdError + 'static;

    fn get_param(
        &self,
        param_name: &str,
    ) -> impl Future<Output = Result<Option<JsonData>, Self::Error>> + Send;

    fn set_param(
        &self,
        param_name: &str,
        param_value: JsonData,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send;
}

pub async fn previous_report<Repository>(
    repository: &Repository,
) -> Result<Option<ROIReport>, Repository::Error>
where
    Repository: ReportRepository,
{
    let Some(parameter) = repository.get_param(REPORT_PARAM).await? else {
        return Ok(None);
    };
    Ok(serde_json::to_vec(&parameter)
        .and_then(|bytes| serde_json::from_slice::<ROIReport>(&bytes))
        .ok())
}

pub async fn cache_estimated_pull<Repository>(
    repository: &Repository,
    key: Option<&str>,
    record: &ROIPullRecord,
    previous: Option<&ROIPullRecord>,
) -> Result<(), CacheWriteError<Repository::Error>>
where
    Repository: ReportRepository,
{
    let Some(key) = key else {
        return Ok(());
    };
    if record.estimate.status != ROIEstimateStatus::Estimated {
        return Ok(());
    }
    let unchanged = previous.is_some_and(|previous| {
        previous.profile_email == record.profile_email && previous.emails == record.emails
    });
    if unchanged {
        return Ok(());
    }
    let value = serde_json::to_vec(record)
        .and_then(|bytes| serde_json::from_slice::<JsonData>(&bytes))
        .map_err(CacheWriteError::Encode)?;
    repository
        .set_param(&format!("{PULL_PARAM_PREFIX}{key}"), value)
        .await
        .map_err(CacheWriteError::Store)
}
