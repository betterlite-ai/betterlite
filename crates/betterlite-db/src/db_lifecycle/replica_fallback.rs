use std::fmt::Display;

use tracing::{info, warn};

// the reader is opt-in: a working deployment keeps serving traffic on the writer alone when it cannot be built
#[must_use]
pub fn initialize_reader<Client, Failure>(
    reader: Option<Result<Client, Failure>>,
    token_label: Option<&str>,
) -> Option<Client>
where
    Failure: Display,
{
    match reader {
        None => None,
        Some(Ok(client)) => {
            let Some(label) = token_label else {
                info!("PrismaClient: read-replica routing enabled via DATABASE_URL_READ_REPLICA");
                return Some(client);
            };
            info!(
                "PrismaClient: read-replica routing enabled via DATABASE_URL_READ_REPLICA (with {label} auto-refresh)"
            );
            Some(client)
        }
        Some(Err(failure)) => {
            warn!(
                "Failed to initialize read replica Prisma client: {failure}. Falling back to writer-only mode (no read routing) until proxy restart."
            );
            None
        }
    }
}
