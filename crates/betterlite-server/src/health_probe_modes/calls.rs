use super::error::HealthProbeError;
use super::evaluation::EvaluationCallParams;
use super::logging::HealthProbeLogging;
use super::mode::HealthProbeMode;
use super::params::HealthProbeParams;
use betterlite_models::decisions::Json;
use betterlite_ocr::ocr_document::OcrDocument;
use indexmap::IndexMap;
use secrecy::SecretString;
use std::collections::BTreeMap;
use std::sync::Arc;
// the provider parameters of one call, without the api key and the logging object
pub struct ProbeCall {
    pub parameters: IndexMap<String, Json>,
    pub api_key: Option<SecretString>,
    pub logging: Option<Arc<dyn HealthProbeLogging>>,
}
pub struct AudioTranscriptionCall {
    pub parameters: IndexMap<String, Json>,
    pub file: Vec<u8>,
    pub api_key: Option<SecretString>,
    pub logging: Option<Arc<dyn HealthProbeLogging>>,
}
pub struct ImageEditCall {
    pub parameters: IndexMap<String, Json>,
    pub image: Vec<u8>,
    pub prompt: &'static str,
    pub api_key: Option<SecretString>,
    pub logging: Option<Arc<dyn HealthProbeLogging>>,
}
pub struct RealtimeCall {
    pub model: String,
    pub custom_llm_provider: String,
    pub api_base: Option<String>,
    pub api_key: Option<SecretString>,
    pub api_version: Option<String>,
    pub model_params: IndexMap<String, Json>,
    pub logging: Option<Arc<dyn HealthProbeLogging>>,
}
pub struct OcrCall {
    pub parameters: IndexMap<String, Json>,
    pub document: OcrDocument,
    pub api_key: Option<SecretString>,
    pub logging: Option<Arc<dyn HealthProbeLogging>>,
}
pub struct EvaluationCall {
    pub params: EvaluationCallParams,
    pub logging: Option<Arc<dyn HealthProbeLogging>>,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HealthProbeReply {
    pub hidden_headers: BTreeMap<String, String>,
}
pub(crate) fn probe_call(
    parameters: IndexMap<String, Json>,
    params: &HealthProbeParams,
) -> ProbeCall {
    ProbeCall {
        parameters,
        api_key: params.api_key.clone(),
        logging: params.litellm_logging_obj.clone(),
    }
}
pub(crate) fn ensure_absent(
    parameters: &IndexMap<String, Json>,
    key: &'static str,
    mode: HealthProbeMode,
) -> Result<(), HealthProbeError> {
    if parameters.contains_key(key) {
        return Err(HealthProbeError::RepeatedParameter {
            key,
            mode: mode.name(),
        });
    }
    Ok(())
}
pub(crate) fn add_parameter(
    parameters: &mut IndexMap<String, Json>,
    key: &'static str,
    value: Json,
    mode: HealthProbeMode,
) -> Result<(), HealthProbeError> {
    ensure_absent(parameters, key, mode)?;
    parameters.insert(key.to_owned(), value);
    Ok(())
}
