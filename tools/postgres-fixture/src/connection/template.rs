use sha2::{Digest as _, Sha256};
use tokio::task::JoinSet;
use tokio_postgres::Client;

use super::cleanup::{close_after, combine_errors, drain_drivers, timed};
use super::operations::{Work, connect, create_resource, fill_resource};
use super::resource::Resource;
use super::{ConnectionOptions, DatabaseNameGenerator as _, RandomDatabaseName, quote_identifier};
use crate::error::FixtureError;

mod tests;

fn template_name(statements: &[String]) -> (String, i64) {
    let digest = <[u8; 32]>::from(Sha256::digest(statements.concat()));
    let [k0, k1, k2, k3, k4, k5, k6, k7, ..] = digest;
    let key = i64::from_be_bytes([k0, k1, k2, k3, k4, k5, k6, k7]);
    (format!("betterlite_template_{key:016x}"), key)
}

pub(super) async fn template(
    admin: &ConnectionOptions,
    statements: &[String],
) -> Result<String, FixtureError> {
    let (name, key) = template_name(statements);
    let mut drivers = JoinSet::new();
    let client = connect(admin, &mut drivers).await?;
    let published = publish(&client, admin, &name, key, statements).await;
    drop(client);
    combine_errors(published, drain_drivers(&mut drivers).await)?;
    Ok(name)
}

async fn publish(
    client: &Client,
    admin: &ConnectionOptions,
    name: &str,
    key: i64,
    statements: &[String],
) -> Result<(), FixtureError> {
    if exists(client, name).await? {
        return Ok(());
    }
    // the lock belongs to this session, so a builder that dies releases it with its connection
    timed(
        "lock template",
        client.execute("SELECT pg_advisory_lock($1)", &[&key]),
    )
    .await?
    .map_err(FixtureError::Database)?;
    if exists(client, name).await? {
        return Ok(());
    }
    // a template is built under a random name and renamed once complete, so no clone ever copies half of one
    let unpublished = RandomDatabaseName.next_name();
    let quoted = quote_identifier(&unpublished);
    let statement = format!("CREATE DATABASE {quoted}");
    let fixture = create_resource(admin, Resource(unpublished), &statement).await?;
    let built = fill_resource(fixture, Work::Statements(statements)).await?;
    let target = quote_identifier(name);
    let rename = format!(
        "ALTER DATABASE {quoted} RENAME TO {target}; ALTER DATABASE {target} WITH IS_TEMPLATE true ALLOW_CONNECTIONS false"
    );
    let renamed = timed("publish template", client.batch_execute(&rename))
        .await
        .and_then(|result| result.map_err(FixtureError::Database));
    match renamed {
        Ok(()) => Ok(()),
        Err(error) => Err(close_after(error, built).await),
    }
}

async fn exists(client: &Client, name: &str) -> Result<bool, FixtureError> {
    let row = timed(
        "find template",
        client.query_opt("SELECT 1 FROM pg_database WHERE datname = $1", &[&name]),
    )
    .await?
    .map_err(FixtureError::Database)?;
    Ok(row.is_some())
}
