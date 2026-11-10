use super::contracts::{SupportsExecuteRaw, SupportsRawQueries};
use super::definitions::{RUNTIME_VIEWS, ViewDefinition};
pub const VIEW_NOT_FOUND_MARKERS: [&str; 3] =
    ["does not exist", "no such table", "undefined table"];
pub const VIEW_ALREADY_EXISTS_MARKERS: [&str; 3] =
    ["already exists", "duplicate object", "duplicate table"];
pub async fn create_missing_views<Database>(database: &Database) -> Result<(), Database::Error>
where
    Database: SupportsRawQueries + Sync,
{
    for view in RUNTIME_VIEWS {
        if !relation_exists(database, &view).await? {
            create_view_tolerating_race(database, view.name, view.ddl).await?;
        }
    }
    Ok(())
}
async fn relation_exists<Source>(
    source: &Source,
    view: &ViewDefinition,
) -> Result<bool, Source::Error>
where
    Source: SupportsRawQueries + Sync,
{
    let probe = format!("SELECT 1 FROM \"{}\" LIMIT 1", view.name);
    match source.query_raw(&probe).await {
        Ok(_) => {
            tracing::debug!("{} Exists!", view.name);
            Ok(true)
        }
        Err(error) => {
            let message = error.to_string().to_lowercase();
            if VIEW_NOT_FOUND_MARKERS
                .iter()
                .any(|marker| message.contains(marker))
            {
                return Ok(false);
            }
            Err(error)
        }
    }
}
pub async fn create_view_tolerating_race<Executor>(
    executor: &Executor,
    view_name: &str,
    ddl: &str,
) -> Result<(), Executor::Error>
where
    Executor: SupportsExecuteRaw + Sync,
{
    let failure = match executor.execute_raw(ddl).await {
        Ok(()) => {
            tracing::debug!("{view_name} Created!");
            return Ok(());
        }
        Err(error) => error,
    };
    let message = failure.to_string().to_lowercase();
    if !VIEW_ALREADY_EXISTS_MARKERS
        .iter()
        .any(|marker| message.contains(marker))
    {
        return Err(failure);
    }
    tracing::debug!("{view_name} already created by a concurrent replica");
    Ok(())
}
