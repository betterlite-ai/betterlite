use betterlite_secret_managers::secret_helpers::normalize_nonempty_secret_str;
use secrecy::SecretString;
use tokio::fs::File;
use tokio::io::AsyncReadExt as _;

use super::error::{TokenFileError, WorkloadIdentityError};

// a mounted service account token is a few KiB; anything past this is not one
pub const MAX_TOKEN_FILE_BYTES: usize = 1 << 20;

pub async fn read_subject_token(path: &str) -> Result<SecretString, WorkloadIdentityError> {
    read_bounded(path)
        .await
        .map_err(|source| WorkloadIdentityError::TokenFile {
            path: path.to_owned(),
            source,
        })
}

async fn read_bounded(path: &str) -> Result<SecretString, TokenFileError> {
    let file = File::open(path).await.map_err(TokenFileError::Io)?;
    let limit = u64::try_from(MAX_TOKEN_FILE_BYTES.saturating_add(1)).unwrap_or(u64::MAX);
    let mut bytes = Vec::new();
    file.take(limit)
        .read_to_end(&mut bytes)
        .await
        .map_err(TokenFileError::Io)?;
    if bytes.len() > MAX_TOKEN_FILE_BYTES {
        return Err(TokenFileError::TooLarge {
            path: path.to_owned(),
            limit: MAX_TOKEN_FILE_BYTES,
        });
    }
    let text = String::from_utf8(bytes).map_err(|_not_utf8| TokenFileError::NotUtf8 {
        path: path.to_owned(),
    })?;
    normalize_nonempty_secret_str(Some(&SecretString::from(text))).ok_or_else(|| {
        TokenFileError::Empty {
            path: path.to_owned(),
        }
    })
}
