use super::{ConnectionOptions, quote_identifier};
use crate::error::FixtureError;

#[derive(Debug)]
pub enum Resource {
    Database(String),
    Schema(String),
}

impl Resource {
    pub(super) fn create_statement(&self) -> String {
        match self {
            Self::Database(name) => format!("CREATE DATABASE {}", quote_identifier(name)),
            Self::Schema(name) => format!("CREATE SCHEMA {}", quote_identifier(name)),
        }
    }

    pub(super) fn drop_statement(&self) -> String {
        match self {
            Self::Database(name) => format!("DROP DATABASE IF EXISTS {}", quote_identifier(name)),
            Self::Schema(name) => {
                format!("DROP SCHEMA IF EXISTS {} CASCADE", quote_identifier(name))
            }
        }
    }

    pub(super) fn connection(
        &self,
        admin: &ConnectionOptions,
    ) -> Result<ConnectionOptions, FixtureError> {
        match self {
            Self::Database(name) => Ok(admin.with_database(name)),
            Self::Schema(name) => {
                let mut parsed = admin.parsed_url.clone();
                let parameters = parsed
                    .query_pairs()
                    .filter(|(key, _)| key != "options")
                    .map(|(key, value)| (key.into_owned(), value.into_owned()))
                    .collect::<Vec<_>>();
                let options = format!(
                    "{} -csearch_path={name}",
                    admin.config.get_options().unwrap_or_default()
                );
                parsed
                    .query_pairs_mut()
                    .clear()
                    .extend_pairs(parameters)
                    .append_pair("options", &options);
                ConnectionOptions::from_url(&parsed.as_str().replace('+', "%20"))
            }
        }
    }
}
