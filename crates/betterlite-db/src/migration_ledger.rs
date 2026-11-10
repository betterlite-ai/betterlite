pub mod baseline;
pub mod budget;
pub mod catalog;
pub mod connection;
pub mod environment;
pub mod fixtures;
pub mod inert;
pub mod ledger_reads;
pub mod lock;
pub mod progress;
pub mod recovery;
mod tests;
pub mod timeouts;

pub(crate) fn ledger_table(schema: &str) -> String {
    // The namespace comes from DATABASE_URL, so a quote inside it must not end the identifier
    let escaped = schema.replace('"', "\"\"");
    format!("\"{escaped}\".\"_prisma_migrations\"")
}
