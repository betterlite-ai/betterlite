use betterlite_logging::logging_pre_post::args::{AdditionalArgs, RawBody};
use betterlite_logging::logging_pre_post::env::PrePostEnv;
use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_logging::logging_pre_post::post_call::{PostCall, post_call};
use betterlite_logging::logging_pre_post::pre_call::{PreCall, pre_call};
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

pub const ATRANSCRIPTION_FIELD: &str = "atranscription";
pub const NVCF_FUNCTION_ID_SET: &str = "nvcf_function_id_set";
pub const USE_SSL: &str = "use_ssl";
pub const RECOGNITION_CONFIG: &str = "recognition_config";

// pre_call's additional_args: the endpoint, the async flag, and the recognition dict with the
// function-ID presence and the raw TLS override beside it
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

pub struct PrePostLogger<'call, 'env> {
    pub state: &'call mut LoggingState,
    pub env: &'call PrePostEnv<'env>,
}

impl RivaLogger for PrePostLogger<'_, '_> {
    fn record(&mut self, event: RivaEvent) -> Result<(), PrePostError> {
        match event {
            RivaEvent::PreCall(record) => {
                let call = PreCall {
                    input: Json::Null,
                    model: None,
                    additional_args: AdditionalArgs {
                        api_base: Some(record.api_base),
                        complete_input_dict: Some(RawBody::Object(IndexMap::from([
                            (
                                RECOGNITION_CONFIG.to_owned(),
                                Json::Object(record.recognition_config),
                            ),
                            (
                                NVCF_FUNCTION_ID_SET.to_owned(),
                                Json::Bool(record.nvcf_function_id_set),
                            ),
                            (USE_SSL.to_owned(), record.use_ssl),
                        ]))),
                        passthrough: IndexMap::from([(
                            ATRANSCRIPTION_FIELD.to_owned(),
                            Json::Bool(record.atranscription),
                        )]),
                        ..AdditionalArgs::default()
                    },
                };
                pre_call(self.state, &call, self.env).map(drop)
            }
            RivaEvent::PostCall(record) => {
                let call = PostCall {
                    original_response: record.original_response,
                    input: record.input,
                    additional_args: AdditionalArgs {
                        complete_input_dict: Some(RawBody::Object(record.recognition_config)),
                        ..AdditionalArgs::default()
                    },
                };
                post_call(self.state, &call, self.env);
                Ok(())
            }
        }
    }
}
