use betterlite_config::loader::EnvironmentLookup;
use std::ops::Mul as _;
use std::str::FromStr;
use std::time::Duration;
const MEBIBYTE: f64 = 1_048_576.0_f64;
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OcrSettings {
    pub request_timeout: Duration,
    pub max_download_bytes: u64,
    pub poll_timeout: Duration,
    pub doc_intelligence_api_version: String,
    pub doc_intelligence_dpi: i64,
    pub vertex_project: Option<String>,
    pub vertex_location: Option<String>,
    pub enable_azure_ad_token_refresh: bool,
}
impl Default for OcrSettings {
    fn default() -> Self {
        Self {
            request_timeout: Duration::from_mins(100),
            max_download_bytes: megabytes(50.0_f64),
            poll_timeout: Duration::from_secs(120),
            doc_intelligence_api_version: "2024-11-30".to_owned(),
            doc_intelligence_dpi: 96,
            vertex_project: None,
            vertex_location: None,
            enable_azure_ad_token_refresh: false,
        }
    }
}
impl OcrSettings {
    #[must_use]
    pub fn from_environment<E>(env: &E) -> Self
    where
        E: EnvironmentLookup,
    {
        let defaults = Self::default();
        Self {
            request_timeout: number::<f64>(env, "REQUEST_TIMEOUT")
                .and_then(|seconds| Duration::try_from_secs_f64(seconds).ok())
                .unwrap_or(defaults.request_timeout),
            max_download_bytes: number::<f64>(env, "MAX_IMAGE_URL_DOWNLOAD_SIZE_MB")
                .filter(|size| size.is_finite())
                .map_or(defaults.max_download_bytes, megabytes),
            poll_timeout: number::<i64>(env, "AZURE_OPERATION_POLLING_TIMEOUT")
                .map_or(defaults.poll_timeout, |seconds| {
                    Duration::from_secs(seconds.max(0).unsigned_abs())
                }),
            doc_intelligence_api_version: env
                .lookup("AZURE_DOCUMENT_INTELLIGENCE_API_VERSION")
                .unwrap_or(defaults.doc_intelligence_api_version),
            doc_intelligence_dpi: number(env, "AZURE_DOCUMENT_INTELLIGENCE_DEFAULT_DPI")
                .unwrap_or(defaults.doc_intelligence_dpi),
            vertex_project: defaults.vertex_project,
            vertex_location: defaults.vertex_location,
            enable_azure_ad_token_refresh: defaults.enable_azure_ad_token_refresh,
        }
    }
}
fn number<T>(env: &dyn EnvironmentLookup, name: &str) -> Option<T>
where
    T: FromStr,
{
    betterlite_env::number::<T>(name, &env.lookup(name).unwrap_or_default())
        .ok()
        .flatten()
}
// Python's int(size * 1024 * 1024) truncates toward zero and saturates at zero for a negative size
fn megabytes(size: f64) -> u64 {
    let bytes = size.mul(MEBIBYTE);
    if !bytes.is_finite() || bytes <= 0.0_f64 {
        return 0;
    }
    format!("{:.0}", bytes.trunc()).parse().unwrap_or(u64::MAX)
}
mod tests;
