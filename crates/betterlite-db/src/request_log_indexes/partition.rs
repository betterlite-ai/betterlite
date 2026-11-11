use crate::request_log_index_catalog::catalog::{Relation, regclass_name};
use crate::request_log_index_catalog::definitions::RequestLogIndex;

use super::IndexBuilder;
use super::adopt::adopt_equivalent;
use super::database::{IndexDatabase, IndexRef};
use super::leaf::build_leaf;
use super::locks::with_bounded_lock;
use super::report::report_second_copies;

// Mirrors build_index_on_partitioned_table: a metadata-only parent, recursive children, then ATTACH
pub(crate) async fn build_on_partitioned_table<D>(
    builder: &IndexBuilder<'_, D>,
    index: RequestLogIndex<'_>,
    table: Option<&str>,
    name: Option<&str>,
) -> Result<bool, D::Error>
where
    D: IndexDatabase,
{
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
    let target = IndexRef {
        schema: builder.schema,
        table: parent_table,
        name: parent_index,
    };
    let children = builder.database.children_without_the_index(target).await?;
    for child in &children {
        if !attach_child_index(builder, parent_index, child, index).await? {
            return Ok(false);
        }
    }
    finish_partitioned(builder, parent_table, parent_index, index).await
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
        || async {
            if adopt_equivalent(builder, table, name, index).await? {
                return Ok(true);
            }
            builder
                .database
                .execute(&parent_statement(builder.schema, table, name, index))
                .await?;
            Ok(true)
        },
    )
    .await
}

async fn attach_child_index<D>(
    builder: &IndexBuilder<'_, D>,
    parent_index: &str,
    child: &Relation,
    index: RequestLogIndex<'_>,
) -> Result<bool, D::Error>
where
    D: IndexDatabase,
{
    let child_index = index.partition_index_name(&child.name);
    let built = if child.partitioned {
        Box::pin(build_on_partitioned_table(
            builder,
            index,
            Some(&child.name),
            Some(&child_index),
        ))
        .await?
    } else {
        build_leaf(builder, &child.name, &child_index, index).await?
    };
    if !built {
        return Ok(false);
    }
    let statement = format!(
        "ALTER INDEX {} ATTACH PARTITION {}",
        regclass_name(builder.schema, parent_index),
        regclass_name(&child.schema, &child_index)
    );
    let what = format!("attaching {child_index}");
    let child_name = child.name.clone();
    with_bounded_lock(builder, &what, || async {
        builder.database.execute(&statement).await?;
        tracing::info!("Attached index {child_index} on partition {child_name} to {parent_index}");
        Ok(true)
    })
    .await
}

async fn finish_partitioned<D>(
    builder: &IndexBuilder<'_, D>,
    table: &str,
    name: &str,
    index: RequestLogIndex<'_>,
) -> Result<bool, D::Error>
where
    D: IndexDatabase,
{
    let final_state = builder.database.index_state(builder.schema, name).await?;
    if final_state.is_none_or(|state| !state.valid) {
        return Ok(false);
    }
    report_second_copies(builder, table, name, index, false).await?;
    Ok(true)
}

fn parent_statement(schema: &str, table: &str, name: &str, index: RequestLogIndex<'_>) -> String {
    format!(
        "CREATE INDEX IF NOT EXISTS \"{}\" ON ONLY {} {}",
        name.replace('"', "\"\""),
        regclass_name(schema, table),
        index.definition
    )
}
