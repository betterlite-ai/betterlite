use std::time::Duration;

use betterlite_db::database_views::contracts::SupportsRawQueries;
use betterlite_db::database_views::definitions::OFFLINE_VIEWS;
use betterlite_db_admin::command_shell::{DbAdminFuture, DbAdminService, ResolvedConfig};
use betterlite_db_admin::commands::{DbAdminCommand, DbAdminError, DbAdminOutcome, ViewsError};
use secrecy::ExposeSecret as _;
use tokio::task::JoinSet;
use tokio::time::timeout;
use tokio_postgres::{Client, Config, NoTls};

// db_scripts/create_views.py passes http timeout 60000 to Prisma, whose unit is seconds (60000 s)
pub(crate) const OFFLINE_TIMEOUT: Duration = Duration::from_mins(1000);

pub(crate) struct DatabaseViewsService;

impl DbAdminService for DatabaseViewsService {
    fn run(&self, _command: DbAdminCommand, config: ResolvedConfig) -> DbAdminFuture<'_> {
        Box::pin(async move {
            let mut connections = JoinSet::new();
            match connect(config.database_url.expose_secret(), &mut connections).await {
                Ok(client) => offline_views(&client, OFFLINE_TIMEOUT).await,
                Err(source) => Err(DbAdminError::Views {
                    lines: Vec::new(),
                    source,
                }),
            }
        })
    }
}

async fn connect(
    url: &str,
    connections: &mut JoinSet<Result<(), tokio_postgres::Error>>,
) -> Result<Client, ViewsError> {
    let config: Config = url.parse().map_err(|source| ViewsError::Url { source })?;
    let (client, connection) = timeout(OFFLINE_TIMEOUT, config.connect(NoTls))
        .await
        .map_err(|_elapsed| ViewsError::Timeout {
            seconds: OFFLINE_TIMEOUT.as_secs(),
        })?
        .map_err(|source| ViewsError::Connect { source })?;
    connections.spawn(connection);
    Ok(client)
}

// any probe failure, a timeout included, attempts the CREATE; a CREATE failure is fatal
pub(crate) async fn offline_views<Database>(
    database: &Database,
    deadline: Duration,
) -> Result<DbAdminOutcome, DbAdminError>
where
    Database: SupportsRawQueries + Sync,
{
    let mut lines = Vec::new();
    for view in OFFLINE_VIEWS {
        let probe = format!("SELECT 1 FROM \"{}\" LIMIT 1", view.name);
        if matches!(
            timeout(deadline, database.query_raw(&probe)).await,
            Ok(Ok(_))
        ) {
            lines.push(format!("{} Exists!", view.name));
            continue;
        }
        match timeout(deadline, database.execute_raw(view.ddl)).await {
            Ok(Ok(())) => lines.push(format!("{} Created!", view.name)),
            Ok(Err(error)) => {
                let message = error.to_string();
                let source = ViewsError::Database { message };
                return Err(DbAdminError::Views { lines, source });
            }
            Err(_elapsed) => {
                let source = ViewsError::Timeout {
                    seconds: deadline.as_secs(),
                };
                return Err(DbAdminError::Views { lines, source });
            }
        }
    }
    Ok(DbAdminOutcome::Views { lines })
}

mod tests;
