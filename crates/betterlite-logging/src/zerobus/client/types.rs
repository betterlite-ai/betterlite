use secrecy::SecretString;

// Zerobus accepts a retry of these and asks the caller to try later
pub const RETRYABLE_INGEST_STATUS_CODES: [u16; 6] = [408, 429, 500, 502, 503, 504];

pub const TOKEN_REFRESH_LEEWAY_SECONDS: f64 = 60.0;

#[derive(Clone, Debug)]
pub struct ZerobusConnection {
    pub workspace_url: String,
    pub workspace_id: String,
    pub server_endpoint: String,
    pub client_id: String,
    pub client_secret: SecretString,
    pub table_name: String,
}

#[derive(Clone, Debug)]
pub struct ZerobusAccessToken {
    pub value: SecretString,
    pub expires_at: f64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ZerobusIngestFailure {
    pub detail: String,
    pub retryable: bool,
}
