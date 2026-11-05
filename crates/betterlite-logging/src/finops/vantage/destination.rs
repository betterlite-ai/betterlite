use betterlite_http::proxy::multipart::MultipartPart;
use secrecy::{ExposeSecret as _, SecretString};

use crate::finops::vantage::columns::strip_unsupported_columns;
use crate::finops::vantage::error::DeliveryError;
use crate::finops::vantage::records::{is_blank, split_records};
use crate::finops::vantage::sender::{CSV_FIELD_NAME, CSV_MEDIA_TYPE, CsvSender, MultipartPost};
use crate::finops::vantage::settings::{DEFAULT_BASE_URL, SettingsError, VantageSettings};
use crate::finops::vantage::upload_batches::upload_batched;

pub const MAX_ROWS_PER_UPLOAD: usize = 10_000;
pub const MAX_BYTES_PER_UPLOAD: usize = 2 * 1024 * 1024;

pub struct VantageDestination<Sender> {
    pub(super) sender: Sender,
    api_key: SecretString,
    integration_token: SecretString,
    base_url: String,
}

impl<Sender: CsvSender> VantageDestination<Sender> {
    pub fn new(sender: Sender, settings: VantageSettings) -> Result<Self, SettingsError> {
        let api_key = settings
            .api_key
            .filter(|key| !key.expose_secret().is_empty())
            .ok_or(SettingsError::MissingApiKey)?;
        let integration_token = settings
            .integration_token
            .filter(|token| !token.expose_secret().is_empty())
            .ok_or(SettingsError::MissingIntegrationToken)?;
        let base_url = settings
            .base_url
            .unwrap_or_else(|| DEFAULT_BASE_URL.to_owned());
        Ok(Self {
            sender,
            api_key,
            integration_token,
            base_url,
        })
    }

    pub async fn deliver(
        &self,
        content: &[u8],
        filename: &str,
    ) -> Result<(), DeliveryError<Sender::Error>> {
        if content.is_empty() {
            tracing::debug!("Vantage destination: empty content, skipping upload");
            return Ok(());
        }
        let stripped = strip_unsupported_columns(content).map_err(DeliveryError::Columns)?;
        let records = split_records(&stripped);
        let data_rows = records
            .iter()
            .skip(1)
            .filter(|record| !is_blank(record))
            .count();
        if stripped.len() <= MAX_BYTES_PER_UPLOAD && data_rows <= MAX_ROWS_PER_UPLOAD {
            return self.upload_csv(&stripped, filename).await;
        }
        upload_batched(self, &records, filename).await
    }

    pub(super) async fn upload_csv(
        &self,
        csv: &[u8],
        filename: &str,
    ) -> Result<(), DeliveryError<Sender::Error>> {
        let url = format!(
            "{}/v2/integrations/{}/costs.csv",
            self.base_url,
            self.integration_token.expose_secret()
        );
        let post = MultipartPost {
            url: SecretString::from(url),
            authorization: SecretString::from(format!("Bearer {}", self.api_key.expose_secret())),
            file: MultipartPart {
                name: CSV_FIELD_NAME.to_owned(),
                file_name: Some(filename.to_owned()),
                content_type: Some(CSV_MEDIA_TYPE.to_owned()),
                data: csv.to_vec(),
            },
        };
        let status = self
            .sender
            .post_multipart(post)
            .await
            .map_err(DeliveryError::Send)?;
        if !(200..300).contains(&status) {
            return Err(DeliveryError::Status { status });
        }
        tracing::debug!(
            "Vantage destination: uploaded {} bytes ({filename})",
            csv.len()
        );
        Ok(())
    }
}
