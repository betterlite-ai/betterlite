use std::error::Error;
use std::future::Future;

use betterlite_http::proxy::multipart::MultipartPart;
use secrecy::SecretString;

pub const CSV_FIELD_NAME: &str = "csv";
pub const CSV_MEDIA_TYPE: &str = "text/csv";

#[derive(Debug)]
pub struct MultipartPost {
    pub url: SecretString,
    pub authorization: SecretString,
    pub file: MultipartPart,
}

pub trait CsvSender: Send + Sync {
    type Error: Error + Send + Sync + 'static;

    fn post_multipart(
        &self,
        post: MultipartPost,
    ) -> impl Future<Output = Result<u16, Self::Error>> + Send;
}
