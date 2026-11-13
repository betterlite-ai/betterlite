use std::error::Error;

use secrecy::SecretString;

use crate::database_replica::availability::{Availability, ReadTarget};
use crate::database_replica::client::{Attribute, DatabaseClient};

pub const MODEL_READ_METHODS: [&str; 9] = [
    "find_first",
    "find_first_or_raise",
    "find_many",
    "find_unique",
    "find_unique_or_raise",
    "count",
    "group_by",
    "query_first",
    "query_raw",
];

pub const TOP_LEVEL_READ_METHODS: [&str; 2] = ["query_first", "query_raw"];

#[derive(Debug, thiserror::Error)]
pub enum RouteError<ClientError>
where
    ClientError: Error + Send + Sync + 'static,
{
    #[error("the database has no attribute named {name}")]
    MissingAttribute { name: String },
    #[error("the database attribute lookup failed")]
    Lookup {
        #[source]
        source: ClientError,
    },
}

pub enum Route<'route, Accessor> {
    Direct(Accessor),
    Routed(RoutedActions<'route, Accessor>),
}

impl<Accessor> Route<'_, Accessor> {
    #[must_use]
    pub fn select(&self, method: &str) -> &Accessor {
        match self {
            Route::Direct(value) => value,
            Route::Routed(actions) => actions.select(method),
        }
    }
}

pub struct RoutedActions<'route, Accessor> {
    writer: Accessor,
    reader: Accessor,
    availability: &'route Availability,
}

impl<Accessor> RoutedActions<'_, Accessor> {
    // the flag is read here on every lookup, so a degraded reader reroutes without rebuilding the accessor
    #[must_use]
    pub fn select(&self, method: &str) -> &Accessor {
        if MODEL_READ_METHODS.contains(&method) && !self.availability.reader_unavailable() {
            &self.reader
        } else {
            &self.writer
        }
    }
}

pub struct RoutingDatabase<Client> {
    pub writer: Client,
    pub reader: Client,
    pub availability: Availability,
    pub reader_url: Option<SecretString>,
}

impl<Client> RoutingDatabase<Client>
where
    Client: DatabaseClient,
{
    #[must_use]
    pub fn read_target(&self) -> ReadTarget {
        if self.availability.reader_unavailable() {
            ReadTarget::Writer
        } else {
            ReadTarget::Reader
        }
    }

    pub fn attribute(
        &self,
        name: &str,
    ) -> Result<Route<'_, Client::Attribute>, RouteError<Client::Error>> {
        if TOP_LEVEL_READ_METHODS.contains(&name) {
            let target = match self.read_target() {
                ReadTarget::Reader => &self.reader,
                ReadTarget::Writer => &self.writer,
            };
            return match target
                .attribute(name)
                .map_err(|source| RouteError::Lookup { source })?
            {
                Some(Attribute::Direct(value) | Attribute::Model(value)) => {
                    Ok(Route::Direct(value))
                }
                None => Err(RouteError::MissingAttribute {
                    name: name.to_owned(),
                }),
            };
        }
        match self
            .writer
            .attribute(name)
            .map_err(|source| RouteError::Lookup { source })?
        {
            None => Err(RouteError::MissingAttribute {
                name: name.to_owned(),
            }),
            Some(Attribute::Direct(value)) => Ok(Route::Direct(value)),
            Some(Attribute::Model(writer)) => match self.reader.attribute(name) {
                Ok(Some(Attribute::Direct(reader) | Attribute::Model(reader))) => {
                    Ok(Route::Routed(RoutedActions {
                        writer,
                        reader,
                        availability: &self.availability,
                    }))
                }
                Ok(None) => Ok(Route::Direct(writer)),
                Err(source) => Err(RouteError::Lookup { source }),
            },
        }
    }
}
