use std::collections::BTreeMap;

use betterlite_config::tls::Verification;
use secrecy::SecretString;

use super::wire::DetectorParams;

mod tests;

#[derive(Debug, Clone)]
pub struct IbmDetectorConfig {
    pub guardrail_name: String,
    pub auth_token: SecretString,
    pub base_url: String,
    pub api_url: String,
    pub detector_id: String,
    pub is_detector_server: bool,
    pub detector_params: DetectorParams,
    pub extra_headers: BTreeMap<String, String>,
    pub score_threshold: Option<f64>,
    pub block_on_detection: bool,
    pub ssl_verification: Verification,
}
