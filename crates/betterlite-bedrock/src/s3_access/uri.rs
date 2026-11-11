use super::{S3_SCHEME, S3AccessError};

pub fn parse_s3_uri(s3_uri: &str) -> Result<(String, String), S3AccessError> {
    let Some(path) = s3_uri.strip_prefix(S3_SCHEME) else {
        return Err(S3AccessError::InvalidS3Uri {
            uri: s3_uri.to_owned(),
        });
    };
    let Some((bucket, key)) = path.split_once('/') else {
        return Err(S3AccessError::InvalidS3Uri {
            uri: s3_uri.to_owned(),
        });
    };
    Ok((bucket.to_owned(), key.to_owned()))
}
