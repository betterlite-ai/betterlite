use crate::proxy_helpers::checks::{self, ProxyCheckError};

pub fn _get_prisma_client_or_none<'database, Client>(
    database: Option<&'database Client>,
) -> Option<&'database Client> {
    // absent means in-memory mode, which get_prisma_client_or_throw turns into status 500 detail {"error": message}
    database
}

// ast-grep-ignore: no-one-line-functions
pub fn get_prisma_client_or_throw<'database, Client>(
    database: Option<&'database Client>,
    message: &str,
) -> Result<&'database Client, ProxyCheckError> {
    checks::prisma_client_or_throw(database, message)
}

#[cfg(test)]
mod tests;
