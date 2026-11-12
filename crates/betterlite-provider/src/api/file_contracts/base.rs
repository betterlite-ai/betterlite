use super::config::CompleteUrlRequest;
use super::error::FileContractError;

pub trait BaseConfig {
    fn get_complete_url(
        &self,
        request: CompleteUrlRequest<'_>,
    ) -> Result<String, FileContractError> {
        request
            .api_base
            .map(str::to_owned)
            .ok_or(FileContractError::ApiBaseRequired)
    }
}
