use std::error::Error;
use std::sync::atomic::Ordering;
use std::time::Duration;

use secrecy::{ExposeSecret as _, SecretString};
use tracing::{info, warn};

use crate::database_refresh::handle::{PoolFuture, PoolTransport};
use crate::database_replica::client::{DatabaseClient, DatabaseLifecycle};
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

    fn connect(&self, timeout: Option<Duration>) -> PoolFuture<'_, (), Client::Error> {
        Box::pin(async move {
            let writer_flag = &self.availability.writer_unavailable;
            let reader_flag = &self.availability.reader_unavailable;
            let writer_error = if self.writer.is_connected() {
                None
            } else {
                self.writer.connect(timeout).await.err()
            };
            if writer_error.is_none() {
                writer_flag.store(false, Ordering::SeqCst);
                info!("[writer] DB connected");
            }
            let reader_error = if self.reader.is_connected() {
                None
            } else {
                self.reader.connect(timeout).await.err()
            };
            if reader_error.is_none() {
                reader_flag.store(false, Ordering::SeqCst);
                info!("[reader] DB connected");
            }
            match (writer_error, reader_error) {
                (None, None) => Ok(()),
                (Some(error), Some(_reader_error)) => Err(error),
                (None, Some(error)) => {
                    reader_flag.store(true, Ordering::SeqCst);
                    warn!(
                        "Failed to connect to read replica DB: {error}. Falling back to the writer for reads until the reader is reachable."
                    );
                    Ok(())
                }
                (Some(error), None) => {
                    writer_flag.store(true, Ordering::SeqCst);
                    warn!(
                        "Failed to connect to primary (writer) DB: {error}. Serving reads from the read replica; writes will fail until the writer recovers."
                    );
                    Ok(())
                }
            }
        })
    }

    fn disconnect(&self, timeout: Option<Duration>) -> PoolFuture<'_, (), Client::Error> {
        Box::pin(async move {
            let writer = disconnect_client(&self.writer, timeout).await;
            let reader = disconnect_client(&self.reader, timeout).await;
            match (writer, reader) {
                (Err(error), _) => Err(error),
                (Ok(()), outcome) => outcome,
            }
        })
    }

    fn recreate<'database>(
        &'database self,
        url: &'database SecretString,
        transport: Option<&'database PoolTransport>,
        expected_generation: Option<u64>,
    ) -> PoolFuture<'database, bool, Client::Error> {
        Box::pin(async move {
            let writer_recreated = self
                .writer
                .recreate(url, transport, expected_generation)
                .await?;
            if !writer_recreated {
                return Ok(false);
            }
            self.availability
                .writer_unavailable
                .store(false, Ordering::SeqCst);
            match recreate_reader(self, transport).await {
                Ok(()) => self
                    .availability
                    .reader_unavailable
                    .store(false, Ordering::SeqCst),
                Err(error) => {
                    self.availability
                        .reader_unavailable
                        .store(true, Ordering::SeqCst);
                    warn!(
                        "Failed to recreate reader Prisma client: {error}. Reads will fall back to the writer until the reader recovers."
                    );
                }
            }
            Ok(true)
        })
    }

    fn start_refresh(&self) -> PoolFuture<'_, (), Client::Error> {
        Box::pin(async move {
            self.writer.start_refresh().await?;
            self.reader.start_refresh().await
        })
    }

    fn stop_refresh(&self) -> PoolFuture<'_, (), Client::Error> {
        Box::pin(async move {
            self.writer.stop_refresh().await?;
            self.reader.stop_refresh().await
        })
    }
}

async fn disconnect_client<Client>(
    client: &Client,
    timeout: Option<Duration>,
) -> Result<(), Client::Error>
where
    Client: DatabaseClient,
{
    match client.disconnect(timeout).await {
        Ok(()) => Ok(()),
        Err(error) => {
            warn!("Error disconnecting Prisma client: {error}");
            Err(error)
        }
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
