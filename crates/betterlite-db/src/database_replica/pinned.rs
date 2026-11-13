use std::sync::atomic::Ordering;

use crate::database_replica::client::{Attribute, DatabaseClient};
use crate::database_replica::routing::{Route, RouteError, RoutingDatabase};

// read-after-write paths pick their database once: a replica can lag the row they just committed
pub enum WriterPinnedClient<'database, Client> {
    Writer(&'database Client),
    Routed(&'database RoutingDatabase<Client>),
}

// a pinned view holds only references, so copying it never copies the database
impl<Client> Clone for WriterPinnedClient<'_, Client> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<Client> Copy for WriterPinnedClient<'_, Client> {}

// a degraded writer keeps the routing wrapper so model loads still read the replica; writes fail anyway
#[must_use]
pub fn pin_writer<Client>(
    database: WriterPinnedClient<'_, Client>,
) -> WriterPinnedClient<'_, Client>
where
    Client: DatabaseClient,
{
    match database {
        WriterPinnedClient::Routed(routing)
            if !routing
                .availability
                .writer_unavailable
                .load(Ordering::SeqCst) =>
        {
            WriterPinnedClient::Writer(&routing.writer)
        }
        WriterPinnedClient::Writer(writer) => WriterPinnedClient::Writer(writer),
        WriterPinnedClient::Routed(routing) => WriterPinnedClient::Routed(routing),
    }
}

// raw SQL writes have no replica fallback, so this ignores the writer flag
#[must_use]
pub const fn writer_wrapper<Client>(database: WriterPinnedClient<'_, Client>) -> &Client {
    match database {
        WriterPinnedClient::Writer(writer) => writer,
        WriterPinnedClient::Routed(routing) => &routing.writer,
    }
}

impl<Client> WriterPinnedClient<'_, Client>
where
    Client: DatabaseClient,
{
    pub fn attribute(
        &self,
        name: &str,
    ) -> Result<Route<'_, Client::Attribute>, RouteError<Client::Error>> {
        match self {
            WriterPinnedClient::Writer(writer) => match writer
                .attribute(name)
                .map_err(|source| RouteError::Lookup { source })?
            {
                Some(Attribute::Direct(value) | Attribute::Model(value)) => {
                    Ok(Route::Direct(value))
                }
                None => Err(RouteError::MissingAttribute {
                    name: name.to_owned(),
                }),
            },
            WriterPinnedClient::Routed(routing) => routing.attribute(name),
        }
    }
}
