pub mod api_call;
pub mod bool_config;
pub mod error;
pub mod error_fallback;
pub mod headers;
pub mod json_header;
pub mod model_provider;
pub mod payload;
pub mod response;
pub mod response_headers;
pub mod settings;
pub mod truncate;

// LiteLLM reports the version it was installed as; betterlite reports the release whose behavior it copies
pub const LITELLM_VERSION: &str = "1.105.1";
pub const MAX_PILLAR_HEADER_VALUE_BYTES: usize = 8 * 1024;
pub const TRUSTED_PILLAR_RESPONSE_HEADERS_METADATA_KEY: &str = "_pillar_response_headers_trusted";

#[path = "pillar/api_call/tests.rs"]
mod api_call_tests;
#[path = "pillar/bool_parsing/tests.rs"]
mod bool_parsing_tests;
#[path = "pillar/encode_json_for_header/tests.rs"]
mod encode_json_for_header_tests;
#[path = "pillar/error_fallback/tests.rs"]
mod error_fallback_tests;
#[path = "pillar/header_construction/tests.rs"]
mod header_construction_tests;
#[path = "pillar/model_extraction/tests.rs"]
mod model_extraction_tests;
#[path = "pillar/payload_construction/tests.rs"]
mod payload_construction_tests;
#[path = "pillar/response_headers/tests.rs"]
mod response_headers_tests;
#[path = "pillar/response_processing/tests.rs"]
mod response_processing_tests;
#[path = "pillar/truncate_evidence_payload/tests.rs"]
mod truncate_evidence_payload_tests;
