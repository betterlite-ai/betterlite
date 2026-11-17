pub const DEFAULT_API_URL: &str = "https://api.pointfive.co/api/v1/ingestion";
pub const RETRYABLE_UPLOAD_STATUS_CODES: [u16; 5] = [429, 500, 502, 503, 504];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PointFiveUploadTarget {
    pub upload_url: String,
    pub object_key: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PointFiveUploadFailure {
    pub detail: String,
    pub retryable: bool,
}
