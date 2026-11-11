use crate::request_log_index_catalog::definitions::RequestLogIndex;

use super::IndexBuilder;
use super::database::{IndexDatabase, IndexRef};
use super::leaf_step::leaf_step;
use super::locks::HANDOVER;
use super::report::report_second_copies;

// Mirrors _build_leaf_index: CONCURRENTLY so writes keep flowing, the catalog read again under the lock
pub(crate) async fn build_leaf<D>(
    builder: &IndexBuilder<'_, D>,
    target: IndexRef<'_>,
    index: RequestLogIndex<'_>,
) -> Result<bool, D::Error>
where
    D: IndexDatabase,
{
    let current = builder
        .database
        .index_state(target.schema, target.name)
        .await?;
    let needs_lock = current
        .as_ref()
        .is_none_or(|state| !state.valid || state.table != target.table);
    if needs_lock {
        let step = Box::pin(leaf_step(builder, target, index));
        if !builder.database.coordinate(step).await? {
            return Ok(false);
        }
        builder.dependencies.pause.pause(HANDOVER).await;
    }
    report_second_copies(builder, target, index, true).await?;
    Ok(true)
}
