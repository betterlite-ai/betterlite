use crate::database_url_policy::url_codec::{
    SplitUrl, UrlSplitError, parse_qsl, urlencode_pairs, urlsplit, urlunsplit,
};

pub const DEFAULT_SCHEMA: &str = "public";

// Mirrors migration_lock.py::migration_environment: DIRECT_URL carries the migration connection while DATABASE_URL picks its schema.
pub fn migration_environment(
    database_url: Option<&str>,
    direct_url: Option<&str>,
) -> Result<Option<String>, UrlSplitError> {
    let Some(url) = database_url else {
        return Ok(None);
    };
    let Some(direct) = direct_url else {
        return Ok(Some(url.to_owned()));
    };
    if url.is_empty() || direct.is_empty() {
        return Ok(Some(url.to_owned()));
    }
    let schema = prisma_schema_param(url)?.unwrap_or_else(|| DEFAULT_SCHEMA.to_owned());
    let parts = urlsplit(direct)?;
    let mut parameters: Vec<(String, String)> = parse_qsl(&parts.query, false)
        .into_iter()
        .filter(|(key, _)| key != "schema")
        .collect();
    parameters.push(("schema".to_owned(), schema));
    let query = urlencode_pairs(&parameters);
    Ok(Some(urlunsplit(&SplitUrl { query, ..parts })))
}

// Mirrors utils.py::_prisma_schema_param: the first nonblank schema query value, or None when the URL names none.
pub fn prisma_schema_param(url: &str) -> Result<Option<String>, UrlSplitError> {
    let parts = urlsplit(url)?;
    Ok(parse_qsl(&parts.query, false)
        .into_iter()
        .find(|(key, _)| key == "schema")
        .map(|(_, value)| value))
}
