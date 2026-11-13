use std::error::Error;
use std::time::Duration;

use secrecy::{ExposeSecret as _, SecretString};
use tracing::{info, warn};

use crate::database_refresh::handle::PoolTransport;
use crate::database_replica::client::{ClientFuture, DatabaseClient, DatabaseLifecycle};
use crate::database_replica::routing::RoutingDatabase;

#[derive(Debug, thiserror::Error)]
pub enum ReaderRecreateError<ClientError>
where
    ClientError: Error + Send + Sync + 'static,
{
    #[error("Failed to generate fresh {label} for read replica")]
    FreshTokenMissing { label: &'static str },
    #[error("DATABASE_URL_READ_REPLICA not set; cannot recreate read replica client")]
    MissingUrl,
    #[error("the read replica credential could not be minted: {source}")]
    Mint {
        #[source]
        source: ClientError,
    },
    #[error("the read replica client could not be recreated: {source}")]
    Recreate {
        #[source]
        source: ClientError,
    },
}

impl<Client> DatabaseLifecycle for RoutingDatabase<Client>
where
    Client: DatabaseClient,
{
    type Error = Client::Error;

    fn is_connected(&self) -> bool {
        self.writer.is_connected()
    }

    fn connect(&self, timeout: Option<Duration>) -> ClientFuture<'_, (), Client::Error> {
        Box::pin(async move {
            let writer_error = attempt_connect(&self.writer, timeout).await;
            if writer_error.is_none() {
                self.availability.mark_writer_recovered();
                info!("[writer] DB connected");
            }
            let reader_error = attempt_connect(&self.reader, timeout).await;
            if reader_error.is_none() {
                self.availability.clear_reader_unavailable();
                info!("[reader] DB connected");
            }
            match (writer_error, reader_error) {
                (None, None) => Ok(()),
                (Some(error), Some(_reader_error)) => Err(error),
                (None, Some(error)) => {
                    self.availability.mark_reader_unavailable();
                    warn!(
                        "Failed to connect to read replica DB: {error}. Falling back to the writer for reads until the reader is reachable."
                    );
                    Ok(())
                }
                (Some(error), None) => {
                    self.availability.mark_writer_unavailable();
                    warn!(
                        "Failed to connect to primary (writer) DB: {error}. Serving reads from the read replica; writes will fail until the writer recovers."
                    );
                    Ok(())
                }
            }
        })
    }

    fn disconnect(&self, timeout: Option<Duration>) -> ClientFuture<'_, (), Client::Error> {
        Box::pin(async move {
            let writer_error = attempt_disconnect(&self.writer, timeout).await;
            let reader_error = attempt_disconnect(&self.reader, timeout).await;
            writer_error.or(reader_error).map_or_else(|| Ok(()), Err)
        })
    }

    fn recreate<'database>(
        &'database self,
        url: &'database SecretString,
        transport: Option<&'database PoolTransport>,
        expected_generation: Option<u64>,
    ) -> ClientFuture<'database, bool, Client::Error> {
        Box::pin(async move {
            let writer_recreated = self
                .writer
                .recreate(url, transport, expected_generation)
                .await?;
            if !writer_recreated {
                return Ok(false);
            }
            self.availability.mark_writer_recovered();
            match recreate_reader(self, transport).await {
                Ok(()) => self.availability.clear_reader_unavailable(),
                Err(error) => {
                    self.availability.mark_reader_unavailable();
                    warn!(
                        "Failed to recreate reader Prisma client: {error}. Reads will fall back to the writer until the reader recovers."
                    );
                }
            }
            Ok(true)
        })
    }

    fn start_refresh(&self) -> ClientFuture<'_, (), Client::Error> {
        Box::pin(async move {
            self.writer.start_refresh().await?;
            self.reader.start_refresh().await
        })
    }

    fn stop_refresh(&self) -> ClientFuture<'_, (), Client::Error> {
        Box::pin(async move {
            self.writer.stop_refresh().await?;
            self.reader.stop_refresh().await
        })
    }
}

async fn attempt_disconnect<Client>(
    client: &Client,
    timeout: Option<Duration>,
) -> Option<Client::Error>
where
    Client: DatabaseClient,
{
    let failed = client.disconnect(timeout).await.err();
    if let Some(error) = &failed {
        warn!("Error disconnecting Prisma client: {error}");
    }
    failed
}

async fn attempt_connect<Client>(
    client: &Client,
    timeout: Option<Duration>,
) -> Option<Client::Error>
where
    Client: DatabaseClient,
{
    if client.is_connected() {
        None
    } else {
        client.connect(timeout).await.err()
    }
}

// a token reader mints its own credential; a password reader reuses the resolved replica URL as it stands
async fn recreate_reader<Client>(
    database: &RoutingDatabase<Client>,
    transport: Option<&PoolTransport>,
) -> Result<(), ReaderRecreateError<Client::Error>>
where
    Client: DatabaseClient,
{
    let url = if database.reader.token_refresh_enabled() {
        match database.reader.minted_url() {
            Ok(Some(url)) if !url.expose_secret().is_empty() => url,
            Ok(_falsey) => {
                return Err(ReaderRecreateError::FreshTokenMissing {
                    label: database.reader.token_label(),
                });
            }
            Err(source) => return Err(ReaderRecreateError::Mint { source }),
        }
    } else {
        let current = database
            .reader_url
            .as_ref()
            .map_or("", secrecy::ExposeSecret::expose_secret);
        if current.is_empty() {
            return Err(ReaderRecreateError::MissingUrl);
        }
        SecretString::from(current.to_owned())
    };
    match database.reader.recreate(&url, transport, None).await {
        Ok(_recreated) => Ok(()),
        Err(source) => Err(ReaderRecreateError::Recreate { source }),
    }
}
