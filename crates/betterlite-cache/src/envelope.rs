pub const RESPONSE_FIELD: &str = "response";
pub const TIMESTAMP_FIELD: &str = "timestamp";

pub mod backend;
pub mod decode;
pub mod facade;
pub mod lookup;
pub mod request;

#[path = "envelope/tests/decode.rs"]
mod decode_tests;
#[path = "envelope/tests/facade.rs"]
mod facade_tests;
#[path = "envelope/tests/limits.rs"]
mod limits_tests;
#[path = "envelope/tests/lookup.rs"]
mod lookup_tests;
#[path = "envelope/tests/support.rs"]
mod support_tests;
#[path = "envelope/tests.rs"]
mod tests;
