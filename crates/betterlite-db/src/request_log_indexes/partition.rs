use crate::request_log_index_catalog::catalog::{Relation, regclass_name};
use crate::request_log_index_catalog::definitions::RequestLogIndex;

use super::IndexBuilder;
use super::adopt::adopt_equivalent;
use super::database::{IndexDatabase, IndexRef};
use super::leaf::build_leaf;
use super::locks::with_bounded_lock;
use super::report::report_second_copies;

pub(crate) struct PartitionTarget<'partition> {
    pub(crate) schema: &'partition str,
    pub(crate) table: Option<&'partition str>,
    pub(crate) name: Option<&'partition str>,
}

// Mirrors build_index_on_partitioned_table: a metadata-only parent, recursive children, then ATTACH
pub(crate) async fn build_on_partitioned_table<D>(
    builder: &IndexBuilder<'_, D>,
    index: RequestLogIndex<'_>,
    partition: PartitionTarget<'_>,
) -> Result<bool, D::Error>
where
    D: IndexDatabase,
{
    let target = IndexRef {
        schema: partition.schema,
        table: partition.table.unwrap_or(index.table),
        name: partition.name.unwrap_or(index.name),
    };
    let existing = builder
        .database
        .index_state(target.schema, target.name)
        .await?;
    if let Some(state) = existing
        .as_ref()
        .filter(|state| state.table != target.table)
    {
        tracing::warn!(
            "Index {} already exists on {} rather than {}, leaving it alone",
            target.name,
            state.table,
            target.table
        );
        return Ok(false);
    }
    if existing.is_none() && !create_or_adopt_parent(builder, target, index).await? {
        return Ok(false);
    }
    let children = builder.database.children_without_the_index(target).await?;
    for child in &children {
        if !attach_child_index(builder, target, child, index).await? {
            return Ok(false);
        }
    }
    finish_partitioned(builder, target, index).await
}

async fn create_or_adopt_parent<D>(
    builder: &IndexBuilder<'_, D>,
    target: IndexRef<'_>,
    index: RequestLogIndex<'_>,
) -> Result<bool, D::Error>
where
    D: IndexDatabase,
{
    with_bounded_lock(
        builder,
        &format!("creating the parent index {}", target.name),
        || async {
            if adopt_equivalent(builder, target, index).await? {
                return Ok(true);
            }
            builder
                .database
                .execute(&parent_statement(target, index))
                .await?;
            Ok(true)
        },
    )
    .await
}

async fn attach_child_index<D>(
    builder: &IndexBuilder<'_, D>,
    parent: IndexRef<'_>,
    child: &Relation,
    index: RequestLogIndex<'_>,
) -> Result<bool, D::Error>
where
    D: IndexDatabase,
{
    let child_index = index.partition_index_name(&child.name);
    let child_target = IndexRef {
        schema: &child.schema,
        table: &child.name,
        name: &child_index,
    };
    let built = if child.partitioned {
        let nested = PartitionTarget {
            schema: &child.schema,
            table: Some(&child.name),
            name: Some(&child_index),
        };
        Box::pin(build_on_partitioned_table(builder, index, nested)).await?
    } else {
        build_leaf(builder, child_target, index).await?
    };
    if !built {
        return Ok(false);
    }
    let statement = format!(
        "ALTER INDEX {} ATTACH PARTITION {}",
        regclass_name(parent.schema, parent.name),
        regclass_name(&child.schema, &child_index)
    );
    let what = format!("attaching {child_index}");
    let child_name = child.name.clone();
    with_bounded_lock(builder, &what, || async {
        builder.database.execute(&statement).await?;
        tracing::info!(
            "Attached index {child_index} on partition {child_name} to {}",
            parent.name
        );
        Ok(true)
    })
    .await
}

async fn finish_partitioned<D>(
    builder: &IndexBuilder<'_, D>,
    target: IndexRef<'_>,
    index: RequestLogIndex<'_>,
) -> Result<bool, D::Error>
where
    D: IndexDatabase,
{
    let final_state = builder
        .database
        .index_state(target.schema, target.name)
        .await?;
    if final_state.is_none_or(|state| !state.valid) {
        return Ok(false);
    }
    report_second_copies(builder, target, index, false).await?;
    Ok(true)
}

fn parent_statement(target: IndexRef<'_>, index: RequestLogIndex<'_>) -> String {
    format!(
        "CREATE INDEX IF NOT EXISTS \"{}\" ON ONLY {} {}",
        target.name.replace('"', "\"\""),
        regclass_name(target.schema, target.table),
        index.definition
    )
}
