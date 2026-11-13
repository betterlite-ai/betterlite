use super::chain::Chain;
use super::message::database_unavailable_message;

pub const SERVICE_UNAVAILABLE_STATUS: u16 = 503;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceUnavailable {
    pub status: u16,
    pub error_type: &'static str,
    pub param: &'static str,
    pub code: &'static str,
    pub message: String,
}

#[must_use]
pub fn service_unavailable_error(chain: &Chain) -> ServiceUnavailable {
    ServiceUnavailable {
        status: SERVICE_UNAVAILABLE_STATUS,
        error_type: "no_db_connection",
        param: "None",
        code: "503",
        message: database_unavailable_message(chain),
    }
}
