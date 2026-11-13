use secrecy::SecretString;

pub mod ahead;
pub mod comparison;
pub mod fixtures;
pub mod guard;
pub mod index_repair;
mod tests;

#[derive(Debug, Clone)]
pub struct MigrationDriftCheck {
    pub database_url: Option<SecretString>,
    pub direct_url: Option<SecretString>,
}
