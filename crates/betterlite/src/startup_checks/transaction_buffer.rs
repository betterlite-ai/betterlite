use betterlite_secret_managers::secret_lookup::text_to_bool;
use betterlite_server::coord_redis_boot::settings::CoordinationGeneralSettings;
use betterlite_server::proxy_request_contracts::coordination_redis::BoolOrText;

use crate::startup_checks::error::StartupCheckError;
use crate::startup_checks::presence::RedisPresence;

#[must_use]
pub fn wants_buffer_redis(settings: &CoordinationGeneralSettings) -> bool {
    match settings.use_redis_transaction_buffer.as_ref() {
        Some(BoolOrText::Bool(value)) => *value,
        Some(BoolOrText::Text(text)) => text_to_bool(text).unwrap_or(false),
        None => false,
    }
}

pub fn check_transaction_buffer(
    settings: &CoordinationGeneralSettings,
    redis: RedisPresence,
) -> Result<(), StartupCheckError> {
    if !wants_buffer_redis(settings) || redis == RedisPresence::Present {
        return Ok(());
    }
    Err(StartupCheckError::TransactionBuffer)
}
