use crate::proxy_helpers::checks::{self, ProxyCheckError};

mod tests;

#[must_use]
// ast-grep-ignore: no-one-line-functions - mirrors LiteLLM's fn _get_prisma_client_or_none over an injected handle
pub const fn get_prisma_client_or_none<Client>(database: Option<&Client>) -> Option<&Client> {
    database
}

// ast-grep-ignore: no-one-line-functions - a missing handle is status 500 with detail {"error": message}
pub fn get_prisma_client_or_throw<'database, Client>(
    database: Option<&'database Client>,
    message: &str,
) -> Result<&'database Client, ProxyCheckError> {
    checks::prisma_client_or_throw(database, message)
}
