use crate::request_log_index_catalog::catalog::{Relation, regclass_name};
use crate::request_log_index_catalog::definitions::RequestLogIndex;

use super::IndexBuilder;
use super::database::{DatabaseFuture, IndexDatabase};
use super::leaf::{adopt_equivalent, build_leaf, report_second_copies};
use super::locks::with_bounded_lock;

// Mirrors build_index_on_partitioned_table: a metadata-only parent, recursive children, then ATTACH
pub(crate) fn build_on_partitioned_table<'partition, D>(
    builder: &'partition IndexBuilder<'_, D>,
    index: RequestLogIndex<'partition>,
    table: Option<&'partition str>,
    name: Option<&'partition str>,
) -> DatabaseFuture<'partition, bool, D::Error>
where
    D: IndexDatabase,
{
    Box::pin(async move {
        let parent_table = table.unwrap_or(index.table);
        let parent_index = name.unwrap_or(index.name);
        let existing = builder
            .database
            .index_state(builder.schema, parent_index)
            .await?;
        if let Some(state) = existing
            .as_ref()
            .filter(|state| state.table != parent_table)
        {
            tracing::warn!(
                "Index {parent_index} already exists on {} rather than {parent_table}, leaving it alone",
                state.table
            );
            return Ok(false);
        }
        if existing.is_none()
            && !create_or_adopt_parent(builder, parent_table, parent_index, index).await?
        {
            return Ok(false);
        }
        let children = builder
            .database
            .children_without_the_index(builder.schema, parent_table, parent_index)
            .await?;
        if !attach_children(builder, &children, parent_index, index).await? {
            return Ok(false);
        }
        let final_state = builder
            .database
            .index_state(builder.schema, parent_index)
            .await?;
        if final_state.is_none_or(|state| !state.valid) {
            return Ok(false);
        }
        report_second_copies(builder, parent_table, parent_index, index, false).await?;
        Ok(true)
    })
}

async fn create_or_adopt_parent<D>(
    builder: &IndexBuilder<'_, D>,
    table: &str,
    name: &str,
    index: RequestLogIndex<'_>,
) -> Result<bool, D::Error>
where
    D: IndexDatabase,
{
    with_bounded_lock(
        builder,
        &format!("creating the parent index {name}"),
        || parent_step(builder, table, name, index),
    )
    .await
}

fn parent_step<'parent, D>(
    builder: &'parent IndexBuilder<'_, D>,
    table: &'parent str,
    name: &'parent str,
    index: RequestLogIndex<'parent>,
) -> DatabaseFuture<'parent, bool, D::Error>
where
    D: IndexDatabase,
{
    Box::pin(async move {
        if adopt_equivalent(builder, table, name, index).await? {
            return Ok(true);
        }
        let statement = format!(
            "CREATE INDEX IF NOT EXISTS \"{}\" ON ONLY {} {}",
            name.replace('"', "\"\""),
            regclass_name(builder.schema, table),
            index.definition
        );
        builder.database.execute(&statement).await?;
        Ok(true)
    })
}

async fn attach_children<D>(
    builder: &IndexBuilder<'_, D>,
    children: &[Relation],
    parent_index: &str,
    index: RequestLogIndex<'_>,
) -> Result<bool, D::Error>
where
    D: IndexDatabase,
{
    for child in children {
        if !attach_child_index(builder, parent_index, child, index).await? {
            return Ok(false);
        }
    }
    Ok(true)
}

fn attach_child_index<'child, D>(
    builder: &'child IndexBuilder<'_, D>,
    parent_index: &'child str,
    child: &'child Relation,
    index: RequestLogIndex<'child>,
) -> DatabaseFuture<'child, bool, D::Error>
where
    D: IndexDatabase,
{
    Box::pin(async move {
        let child_index = index.partition_index_name(&child.name);
        let built = if child.partitioned {
            build_on_partitioned_table(builder, index, Some(&child.name), Some(&child_index))
                .await?
        } else {
            build_leaf(builder, &child.name, &child_index, index).await?
        };
        if !built {
            return Ok(false);
        }
        let what = format!("attaching {child_index}");
        with_bounded_lock(builder, &what, || {
            attach_step(builder, parent_index, child, &child_index)
        })
        .await
    })
}

fn attach_step<'child, D>(
    builder: &'child IndexBuilder<'_, D>,
    parent_index: &'child str,
    child: &'child Relation,
    child_index: &'child str,
) -> DatabaseFuture<'child, bool, D::Error>
where
    D: IndexDatabase,
{
    Box::pin(async move {
        let statement = format!(
            "ALTER INDEX {} ATTACH PARTITION {}",
            regclass_name(builder.schema, parent_index),
            regclass_name(&child.schema, child_index)
        );
        builder.database.execute(&statement).await?;
        tracing::info!(
            "Attached index {child_index} on partition {} to {parent_index}",
            child.name
        );
        Ok(true)
    })
}
