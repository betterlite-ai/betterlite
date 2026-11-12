use std::string::FromUtf8Error;

use betterlite_models::catalog::CatalogError;

#[derive(Debug, thiserror::Error)]
pub(crate) enum NativeApiError {
    #[error("bundled model cost asset: {0}")]
    Asset(#[from] CatalogError),
    #[error("bundled model cost asset is not UTF-8: {0}")]
    NotUtf8(#[from] FromUtf8Error),
}
