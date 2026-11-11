use url::Url;

use super::aggregate::FocusCostEntry;
use super::error::UploadError;
use super::settings::CostManagementSettings;
use super::transport::{PutRequest, Transport};

const SUCCESS_MIN: u16 = 200;
const SUCCESS_MAX: u16 = 299;

pub async fn upload<T>(
    settings: &CostManagementSettings,
    transport: &T,
    entries: &[FocusCostEntry],
) -> Result<(), UploadError<T::Error>>
where
    T: Transport,
{
    let Some(headers) = settings.headers()? else {
        tracing::debug!(
            "Datadog Cost Management: DD_API_KEY or DD_APP_KEY is missing, so no cost entries are uploaded"
        );
        return Ok(());
    };
    let url = Url::parse(&settings.upload_url).map_err(|source| UploadError::Url {
        url: settings.upload_url.clone(),
        source,
    })?;
    let body = serde_json::to_vec(entries).map_err(UploadError::Encode)?;
    let response = transport
        .put(PutRequest { url, headers, body })
        .await
        .map_err(UploadError::Transport)?;
    // httpx raise_for_status accepts only a 2xx answer, so any redirect left unfollowed requeues the batch
    if !(SUCCESS_MIN..=SUCCESS_MAX).contains(&response.status) {
        return Err(UploadError::Status {
            status: response.status,
        });
    }
    tracing::debug!(
        entries = entries.len(),
        status = response.status,
        "Datadog Cost Management: uploaded cost entries"
    );
    Ok(())
}
