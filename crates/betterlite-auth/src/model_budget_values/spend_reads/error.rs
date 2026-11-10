use std::error::Error;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum SpendReadError<Source>
where
    Source: Error + 'static,
{
    #[error("the Redis batch read of model budget spend failed")]
    Redis {
        #[source]
        source: Source,
    },
}
