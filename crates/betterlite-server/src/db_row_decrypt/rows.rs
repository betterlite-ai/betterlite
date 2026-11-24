use std::sync::atomic::Ordering;

use betterlite_db::auxiliary_repository::repository::{
    PrismaClient, PrismaTableRepository, TableError,
};
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::resource_repository::model::ProxyModelBinding;
use betterlite_db::resource_repository::table::{KeyFilter, ResourceActions};
use betterlite_db::resource_rows::model::ProxyModelTable;
use betterlite_redis::config_sync::publish::redis_config_change_publisher;

// WriterPinnedClient: a healthy writer also serves the read; a degraded one leaves the reader routing in place.
#[must_use]
fn pinned_to_writer(client: PrismaClient<'_>) -> PrismaClient<'_> {
    if client
        .availability
        .writer_unavailable
        .load(Ordering::SeqCst)
    {
        return client;
    }
    PrismaClient {
        writer: client.writer,
        reader: client.writer,
        availability: client.availability,
    }
}

// _get_models_from_db: every row through the writer, and None rather than an empty list when the read fails.
#[must_use]
pub async fn read_model_rows(db: Option<PrismaClient<'_>>) -> Option<Vec<ProxyModelTable>> {
    let Some(client) = db else {
        tracing::error!("Error getting new models from DB - no database client");
        return None;
    };
    let repository = PrismaTableRepository::<ProxyModelBinding>::new(
        Some(pinned_to_writer(client)),
        redis_config_change_publisher::<TableError>(None),
        ProxyModelBinding,
    );
    let found = match repository.table(ResourceActions::<ProxyModelBinding>::new) {
        Ok(actions) => actions.as_ref().find::<JsonData>(KeyFilter::All).await,
        Err(error) => Err(TableError::from(error)),
    };
    let rows = match found {
        Ok(rows) => rows,
        Err(error) => {
            tracing::error!("Error getting new models from DB - {error}");
            return None;
        }
    };
    let mut models = Vec::with_capacity(rows.len());
    for row in rows {
        match ProxyModelTable::try_from(row) {
            Ok(model) => models.push(model),
            Err(error) => {
                tracing::error!("Error getting new models from DB - {error}");
                return None;
            }
        }
    }
    Some(models)
}
