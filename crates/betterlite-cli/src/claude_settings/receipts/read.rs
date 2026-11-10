use std::fs;
use std::path::Path;

use super::error::ReceiptsError;
use super::record::ConfigureReceipt;

pub fn read_configure_receipt(
    state_path: &Path,
) -> Result<Option<ConfigureReceipt>, ReceiptsError> {
    if !state_path.exists() {
        return Ok(None);
    }
    let bytes = fs::read(state_path).map_err(|source| ReceiptsError::ReadReceipt {
        path: state_path.to_owned(),
        source,
    })?;
    let receipt = serde_json::from_slice::<ConfigureReceipt>(&bytes).map_err(|source| {
        ReceiptsError::InvalidReceipt {
            path: state_path.to_owned(),
            source,
        }
    })?;
    Ok(Some(receipt))
}
