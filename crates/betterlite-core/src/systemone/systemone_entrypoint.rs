#[path = "systemone_entrypoint/tests/async.rs"]
mod async_tests;
#[path = "systemone_entrypoint/tests/signature.rs"]
mod signature_tests;
#[path = "systemone_entrypoint/tests/sync.rs"]
mod sync_tests;

mod prepare;
mod response;

pub(crate) use prepare::prepare_systemone_call;
pub(crate) use response::systemone_response;
