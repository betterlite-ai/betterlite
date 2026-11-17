use super::{ConnectionOptions, quote_identifier};
use crate::error::FixtureError;
use secrecy::ExposeSecret as _;
use url::Url;

#[derive(Debug)]
pub struct Ownership {
    pub(super) resource: Resource,
    pub(super) admin: Box<ConnectionOptions>,
}

#[derive(Debug)]
pub(super) enum Resource {
    Database(String),
    Schema(String),
}

impl Resource {
    pub(super) fn name(&self) -> &str {
        match self {
            Self::Database(name) | Self::Schema(name) => name,
        }
    }

    pub(super) fn drop_statement(&self) -> String {
        format!(
            "DROP DATABASE IF EXISTS {} WITH (FORCE)",
            quote_identifier(self.name())
        )
    }

    pub(super) fn connection(
        &self,
        admin: &ConnectionOptions,
    ) -> Result<ConnectionOptions, FixtureError> {
        let database = admin.with_database(self.name())?;
        let Self::Schema(name) = self else {
            return Ok(database);
        };
        let mut parsed = Url::parse(database.url.expose_secret())
            .map_err(|source| FixtureError::InvalidUrl { source })?;
        let parameters = parsed
            .query_pairs()
            .filter(|(key, _)| key != "options")
            .map(|(key, value)| (key.into_owned(), value.into_owned()))
            .collect::<Vec<_>>();
        let previous = database.config.get_options().unwrap_or_default();
        let options = format!("{previous} -csearch_path={name}");
        parsed
            .query_pairs_mut()
            .clear()
            .extend_pairs(parameters)
            .append_pair("options", &options);
        let query = parsed.query().unwrap_or_default().replace('+', "%20");
        parsed.set_query(Some(&query));
        ConnectionOptions::from_url(parsed.as_str())
    }
}
