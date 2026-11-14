pub mod config;
pub mod error;
pub mod guardrails;
pub mod inheritance;
pub mod lookup;
pub mod scope;
pub mod types;

mod tests;

#[derive(Debug, Clone)]
pub struct PolicyValidator<Guardrails, Database, Router> {
    pub guardrails: Guardrails,
    pub database: Option<Database>,
    pub router: Option<Router>,
}
