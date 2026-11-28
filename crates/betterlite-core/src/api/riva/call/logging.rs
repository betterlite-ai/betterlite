use betterlite_logging::logging_pre_post::args::{AdditionalArgs, RawBody};
use betterlite_logging::logging_pre_post::context::LogContext;
use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_logging::logging_pre_post::post_call::{PostCall, post_call};
use betterlite_logging::logging_pre_post::pre_call::{PreCall, pre_call};
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

pub const ATRANSCRIPTION_FIELD: &str = "atranscription";
pub const NVCF_FUNCTION_ID_SET: &str = "nvcf_function_id_set";
pub const USE_SSL: &str = "use_ssl";
pub const RECOGNITION_CONFIG: &str = "recognition_config";

// pre_call's additional_args: the endpoint, the async flag, and the recognition dict with the function-ID presence and the raw TLS override beside it
pub struct RivaPreCall {
    pub api_base: String,
    pub atranscription: bool,
    pub recognition_config: IndexMap<String, Json>,
    pub nvcf_function_id_set: bool,
    pub use_ssl: Json,
}

// post_call's additional_args carry only the recognition dict the handler sent
pub struct RivaPostCall {
    pub input: Json,
    pub original_response: Json,
    pub recognition_config: IndexMap<String, Json>,
}

pub enum RivaEvent {
    PreCall(RivaPreCall),
    PostCall(RivaPostCall),
}

pub trait RivaLogger {
    fn record(&mut self, event: RivaEvent) -> Result<(), PrePostError>;
}

// LogContext belongs to betterlite-logging, so its record_pre helper rides a private trait instead of an inherent impl.
trait RivaPreCallLogging {
    fn record_pre(&mut self, record: RivaPreCall) -> Result<(), PrePostError>;
}

impl RivaLogger for LogContext<'_, '_> {
    fn record(&mut self, event: RivaEvent) -> Result<(), PrePostError> {
        match event {
            RivaEvent::PreCall(record) => self.record_pre(record),
            RivaEvent::PostCall(record) => {
                let call = PostCall {
                    original_response: record.original_response,
                    input: record.input,
                    additional_args: AdditionalArgs {
                        api_base: None,
                        headers: None,
                        complete_input_dict: Some(RawBody::Object(record.recognition_config)),
                        request_str: None,
                        openai_sdk: None,
                        passthrough: IndexMap::new(),
                    },
                };
                post_call(self.state, &call, self.env);
                Ok(())
            }
        }
    }
}

impl RivaPreCallLogging for LogContext<'_, '_> {
    fn record_pre(&mut self, record: RivaPreCall) -> Result<(), PrePostError> {
        let fields = IndexMap::from([
            (
                RECOGNITION_CONFIG.to_owned(),
                Json::Object(record.recognition_config),
            ),
            (
                NVCF_FUNCTION_ID_SET.to_owned(),
                Json::Bool(record.nvcf_function_id_set),
            ),
            (USE_SSL.to_owned(), record.use_ssl),
        ]);
        let additional_args = AdditionalArgs {
            api_base: Some(record.api_base),
            headers: None,
            complete_input_dict: Some(RawBody::Object(fields)),
            request_str: None,
            openai_sdk: None,
            passthrough: IndexMap::from([(
                ATRANSCRIPTION_FIELD.to_owned(),
                Json::Bool(record.atranscription),
            )]),
        };
        let call = PreCall {
            input: Json::Null,
            model: None,
            additional_args,
        };
        pre_call(self.state, &call, self.env).map(drop)
    }
}
