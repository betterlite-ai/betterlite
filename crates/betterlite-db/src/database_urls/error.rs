use crate::database_url_policy::url_codec::UrlSplitError;
use crate::database_urls::settings::SettingsError;

#[derive(Debug, thiserror::Error)]
pub enum DatabaseUrlError {
    #[error(transparent)]
    Url {
        #[from]
        source: UrlSplitError,
    },
    #[error(transparent)]
    Settings {
        #[from]
        source: SettingsError,
    },
    #[error("{message}")]
    UnsupportedScheme { message: String },
}
