use std::error::Error;
use std::time::Duration;

use secrecy::SecretString;

use crate::database_refresh::handle::{PoolFuture, PoolTransport};

pub type ClientFuture<'borrowed, T, Error> = PoolFuture<'borrowed, T, Error>;

// Prisma hands out per-model accessors, callable methods and plain values; only the accessor shape is wrapped
pub enum Attribute<Accessor> {
    Direct(Accessor),
    Model(Accessor),
}

pub trait DatabaseLifecycle: Send + Sync {
    type Error: Error + Send + Sync + 'static;

    fn is_connected(&self) -> bool;

    fn connect(&self, timeout: Option<Duration>) -> ClientFuture<'_, (), Self::Error>;

    fn disconnect(&self, timeout: Option<Duration>) -> ClientFuture<'_, (), Self::Error>;

    fn recreate<'database>(
        &'database self,
        url: &'database SecretString,
        transport: Option<&'database PoolTransport>,
        expected_generation: Option<u64>,
    ) -> ClientFuture<'database, bool, Self::Error>;

    fn start_refresh(&self) -> ClientFuture<'_, (), Self::Error>;

    fn stop_refresh(&self) -> ClientFuture<'_, (), Self::Error>;
}

pub trait DatabaseClient: DatabaseLifecycle {
    type Attribute: Clone;

    fn attribute(&self, name: &str) -> Result<Option<Attribute<Self::Attribute>>, Self::Error>;

    fn token_refresh_enabled(&self) -> bool;

    fn token_label(&self) -> &'static str;

    fn minted_url(&self) -> Result<Option<SecretString>, Self::Error>;
}
