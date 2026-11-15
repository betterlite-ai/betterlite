#![cfg(test)]

use std::future::{Future, ready};
use std::sync::{Arc, Mutex};

use betterlite_db::auxiliary_repository::repository::TableError;
use chrono::NaiveDateTime;
use tokio::runtime::Builder;
use tracing::subscriber::with_default;
use tracing::{Level, Subscriber};
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::layer::{Context, Layer};

use super::table::{
    Patch, ToolsetFilter, ToolsetRow, ToolsetTable, ToolsetUpdate, toolset_from_row,
};
use super::{
    MCPToolsetStorageError, MCPToolsetTool, NewMCPToolsetRequest, UpdateMCPToolsetRequest,
    create_mcp_toolset, delete_mcp_toolset, get_mcp_toolset, get_mcp_toolset_by_name,
    list_mcp_toolsets, update_mcp_toolset,
};

mod postgres;

fn clock() -> NaiveDateTime {
    NaiveDateTime::parse_from_str("2026-01-02T03:04:05", "%Y-%m-%dT%H:%M:%S").unwrap()
}

fn tool(server_id: &str, tool_name: &str) -> MCPToolsetTool {
    MCPToolsetTool {
        server_id: server_id.to_owned(),
        tool_name: tool_name.to_owned(),
    }
}

fn toolset_row(toolset_id: &str, toolset_name: &str) -> ToolsetRow {
    ToolsetRow {
        toolset_id: toolset_id.to_owned(),
        toolset_name: toolset_name.to_owned(),
        description: None,
        tools: Some("[]".to_owned()),
        created_at: Some(clock()),
        created_by: None,
        updated_at: Some(clock()),
        updated_by: None,
    }
}

#[derive(Default)]
struct FakeTable {
    rows: Mutex<Vec<ToolsetRow>>,
    filters: Mutex<Vec<(Option<Vec<String>>, bool)>>,
    fail_reads: bool,
}

impl FakeTable {
    fn with_rows(rows: Vec<ToolsetRow>) -> Self {
        Self {
            rows: Mutex::new(rows),
            filters: Mutex::new(Vec::new()),
            fail_reads: false,
        }
    }

    fn failing() -> Self {
        Self {
            rows: Mutex::new(Vec::new()),
            filters: Mutex::new(Vec::new()),
            fail_reads: true,
        }
    }

    fn recorded_filters(&self) -> Vec<(Option<Vec<String>>, bool)> {
        self.filters.lock().unwrap().clone()
    }

    fn find_row<F>(&self, matches: F) -> Option<ToolsetRow>
    where
        F: Fn(&ToolsetRow) -> bool,
    {
        self.rows
            .lock()
            .unwrap()
            .iter()
            .find(|row| matches(row))
            .cloned()
    }

    fn take_row(&self, toolset_id: &str) -> Option<ToolsetRow> {
        let index = self
            .rows
            .lock()
            .unwrap()
            .iter()
            .position(|row| row.toolset_id == toolset_id);
        index.map(|found| self.rows.lock().unwrap().remove(found))
    }
}

impl ToolsetTable for FakeTable {
    fn create(
        &self,
        row: ToolsetRow,
    ) -> impl Future<Output = Result<ToolsetRow, MCPToolsetStorageError>> + Send {
        let stored = ToolsetRow {
            created_at: Some(clock()),
            updated_at: Some(clock()),
            ..row
        };
        self.rows.lock().unwrap().push(stored.clone());
        ready(Ok(stored))
    }

    fn find_unique(
        &self,
        toolset_id: &str,
    ) -> impl Future<Output = Result<Option<ToolsetRow>, MCPToolsetStorageError>> + Send {
        ready(Ok(self.find_row(|row| row.toolset_id == toolset_id)))
    }

    fn find_first_by_name(
        &self,
        toolset_name: &str,
    ) -> impl Future<Output = Result<Option<ToolsetRow>, MCPToolsetStorageError>> + Send {
        ready(Ok(self.find_row(|row| row.toolset_name == toolset_name)))
    }

    fn find_many(
        &self,
        filter: ToolsetFilter,
        use_writer: bool,
    ) -> impl Future<Output = Result<Vec<ToolsetRow>, MCPToolsetStorageError>> + Send {
        let (ids, rows) = {
            let mut filters = self.filters.lock().unwrap();
            match &filter {
                ToolsetFilter::All => filters.push((None, use_writer)),
                ToolsetFilter::Ids(ids) => filters.push((Some(ids.clone()), use_writer)),
            }
            drop(filters);
            let ids = match filter {
                ToolsetFilter::All => None,
                ToolsetFilter::Ids(ids) => Some(ids),
            };
            let rows = self.rows.lock().unwrap().clone();
            (ids, rows)
        };
        if self.fail_reads {
            return ready(Err(MCPToolsetStorageError::Table(TableError::MissingRow)));
        }
        let matched = match ids {
            Some(wanted) => rows
                .into_iter()
                .filter(|row| wanted.contains(&row.toolset_id))
                .collect(),
            None => rows,
        };
        ready(Ok(matched))
    }

    fn update(
        &self,
        toolset_id: &str,
        update: ToolsetUpdate,
    ) -> impl Future<Output = Result<Option<ToolsetRow>, MCPToolsetStorageError>> + Send {
        let mut rows = self.rows.lock().unwrap();
        let result = rows
            .iter_mut()
            .find(|row| row.toolset_id == toolset_id)
            .map(|row| apply_update(row, &update));
        drop(rows);
        ready(Ok(result))
    }

    fn delete(
        &self,
        toolset_id: &str,
    ) -> impl Future<Output = Result<Option<ToolsetRow>, MCPToolsetStorageError>> + Send {
        ready(Ok(self.take_row(toolset_id)))
    }
}

fn apply_update(row: &mut ToolsetRow, update: &ToolsetUpdate) -> ToolsetRow {
    if let Some(name) = &update.toolset_name {
        row.toolset_name.clone_from(name);
    }
    match &update.description {
        Patch::Keep => {}
        Patch::Clear => row.description = None,
        Patch::Set(text) => row.description = Some(text.clone()),
    }
    if let Some(tools) = &update.tools {
        row.tools = Some(tools.clone());
    }
    row.updated_by = Some(update.updated_by.clone());
    row.updated_at = Some(clock());
    row.clone()
}

#[derive(Clone, Default)]
struct WarnCounter(Arc<Mutex<usize>>);

impl WarnCounter {
    fn count(&self) -> usize {
        *self.0.lock().unwrap()
    }
}

impl<S: Subscriber> Layer<S> for WarnCounter {
    fn on_event(&self, event: &tracing::Event<'_>, _: Context<'_, S>) {
        if *event.metadata().level() != Level::WARN {
            return;
        }
        let mut count = self.0.lock().unwrap();
        *count = (*count).saturating_add(1);
    }
}

#[tokio::test]
async fn create_generates_a_uuid_and_stores_the_tools_json() {
    let table = FakeTable::default();
    let request: NewMCPToolsetRequest = serde_json::from_str(
        r#"{"toolset_name": "alpha", "description": "first", "tools": [{"server_id": "s1", "tool_name": "t1"}]}"#,
    )
    .unwrap();
    let created = create_mcp_toolset(&table, &request, "creator")
        .await
        .unwrap();
    assert_eq!(created.toolset_name, "alpha");
    assert_eq!(created.description.as_deref(), Some("first"));
    assert_eq!(created.tools, vec![tool("s1", "t1")]);
    assert_eq!(created.created_by.as_deref(), Some("creator"));
    assert_eq!(created.updated_by.as_deref(), Some("creator"));
    assert_eq!(created.created_at, Some(clock().and_utc()));
    assert_eq!(
        uuid::Uuid::parse_str(&created.toolset_id)
            .unwrap()
            .get_version_num(),
        4
    );
    let stored = table.rows.lock().unwrap();
    assert_eq!(stored.len(), 1);
    assert_eq!(
        stored[0].tools.as_deref(),
        Some(r#"[{"server_id":"s1","tool_name":"t1"}]"#)
    );
}

#[tokio::test]
async fn create_stores_an_empty_tools_json_array_when_the_request_omits_them() {
    let table = FakeTable::default();
    let request: NewMCPToolsetRequest =
        serde_json::from_str(r#"{"toolset_name": "alpha"}"#).unwrap();
    let created = create_mcp_toolset(&table, &request, "creator")
        .await
        .unwrap();
    assert!(created.tools.is_empty());
    assert_eq!(created.description, None);
    assert_eq!(table.rows.lock().unwrap()[0].tools.as_deref(), Some("[]"));
}

#[tokio::test]
async fn get_returns_the_stored_toolset() {
    let table = FakeTable::with_rows(vec![toolset_row("id-1", "alpha")]);
    let found = get_mcp_toolset(&table, "id-1").await.unwrap();
    assert_eq!(
        found.map(|toolset| toolset.toolset_name),
        Some("alpha".to_owned())
    );
}

#[tokio::test]
async fn get_returns_none_for_an_unknown_id() {
    let table = FakeTable::with_rows(vec![toolset_row("id-1", "alpha")]);
    assert!(get_mcp_toolset(&table, "id-2").await.unwrap().is_none());
}

#[tokio::test]
async fn get_by_name_finds_the_matching_row() {
    let table = FakeTable::with_rows(vec![
        toolset_row("id-1", "alpha"),
        toolset_row("id-2", "beta"),
    ]);
    let found = get_mcp_toolset_by_name(&table, "beta").await.unwrap();
    assert_eq!(
        found.map(|toolset| toolset.toolset_id),
        Some("id-2".to_owned())
    );
}

#[tokio::test]
async fn get_by_name_returns_none_for_an_unknown_name() {
    let table = FakeTable::with_rows(vec![toolset_row("id-1", "alpha")]);
    assert!(
        get_mcp_toolset_by_name(&table, "gamma")
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn list_returns_every_toolset_without_a_filter() {
    let table = FakeTable::with_rows(vec![
        toolset_row("id-1", "alpha"),
        toolset_row("id-2", "beta"),
    ]);
    let listed = list_mcp_toolsets(&table, None, false).await.unwrap();
    assert_eq!(listed.len(), 2);
    assert_eq!(table.recorded_filters(), vec![(None, false)]);
}

#[tokio::test]
async fn list_filters_by_the_given_ids() {
    let table = FakeTable::with_rows(vec![
        toolset_row("id-1", "alpha"),
        toolset_row("id-2", "beta"),
    ]);
    let ids = vec!["id-2".to_owned()];
    let listed = list_mcp_toolsets(&table, Some(&ids), false).await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].toolset_id, "id-2");
    assert_eq!(
        table.recorded_filters(),
        vec![(Some(vec!["id-2".to_owned()]), false)]
    );
}

#[test]
fn list_returns_empty_and_warns_on_a_reader_error() {
    let table = FakeTable::failing();
    let warnings = WarnCounter::default();
    let subscriber = tracing_subscriber::registry().with(warnings.clone());
    let runtime = Builder::new_current_thread().enable_all().build().unwrap();
    let listed = with_default(subscriber, || {
        runtime.block_on(list_mcp_toolsets(&table, None, false))
    })
    .unwrap();
    assert!(listed.is_empty());
    assert_eq!(warnings.count(), 1);
}

#[tokio::test]
async fn list_propagates_the_error_when_use_writer_is_true() {
    let table = FakeTable::failing();
    let error = list_mcp_toolsets(&table, None, true).await.unwrap_err();
    assert!(matches!(
        error,
        MCPToolsetStorageError::Table(TableError::MissingRow)
    ));
    assert_eq!(table.recorded_filters(), vec![(None, true)]);
}

#[tokio::test]
async fn update_applies_a_partial_change_to_only_the_set_fields() {
    let mut stored = toolset_row("id-1", "alpha");
    stored.description = Some("first".to_owned());
    stored.tools = Some(r#"[{"server_id":"s1","tool_name":"t1"}]"#.to_owned());
    let table = FakeTable::with_rows(vec![stored]);
    let request: UpdateMCPToolsetRequest =
        serde_json::from_str(r#"{"toolset_id": "id-1", "description": "changed"}"#).unwrap();
    let updated = update_mcp_toolset(&table, &request, "editor")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(updated.toolset_name, "alpha");
    assert_eq!(updated.description.as_deref(), Some("changed"));
    assert_eq!(updated.tools, vec![tool("s1", "t1")]);
    assert_eq!(updated.updated_by.as_deref(), Some("editor"));
    assert_eq!(updated.updated_at, Some(clock().and_utc()));
}

#[tokio::test]
async fn update_treats_a_null_toolset_name_and_tools_as_a_no_op() {
    let mut stored = toolset_row("id-1", "alpha");
    stored.tools = Some(r#"[{"server_id":"s1","tool_name":"t1"}]"#.to_owned());
    let table = FakeTable::with_rows(vec![stored]);
    let request: UpdateMCPToolsetRequest =
        serde_json::from_str(r#"{"toolset_id": "id-1", "toolset_name": null, "tools": null}"#)
            .unwrap();
    let updated = update_mcp_toolset(&table, &request, "editor")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(updated.toolset_name, "alpha");
    assert_eq!(updated.tools, vec![tool("s1", "t1")]);
}

#[tokio::test]
async fn update_clears_the_description_when_it_is_explicitly_null() {
    let mut stored = toolset_row("id-1", "alpha");
    stored.description = Some("first".to_owned());
    let table = FakeTable::with_rows(vec![stored]);
    let request: UpdateMCPToolsetRequest =
        serde_json::from_str(r#"{"toolset_id": "id-1", "description": null}"#).unwrap();
    let updated = update_mcp_toolset(&table, &request, "editor")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(updated.description, None);
}

#[tokio::test]
async fn update_serializes_the_new_tools_as_json() {
    let table = FakeTable::with_rows(vec![toolset_row("id-1", "alpha")]);
    let request: UpdateMCPToolsetRequest = serde_json::from_str(
        r#"{"toolset_id": "id-1", "tools": [{"server_id": "s9", "tool_name": "t9"}]}"#,
    )
    .unwrap();
    let updated = update_mcp_toolset(&table, &request, "editor")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(updated.tools, vec![tool("s9", "t9")]);
    assert_eq!(
        table.rows.lock().unwrap()[0].tools.as_deref(),
        Some(r#"[{"server_id":"s9","tool_name":"t9"}]"#)
    );
}

#[tokio::test]
async fn update_empties_the_tools_when_the_request_sends_an_empty_list() {
    let mut stored = toolset_row("id-1", "alpha");
    stored.tools = Some(r#"[{"server_id":"s1","tool_name":"t1"}]"#.to_owned());
    let table = FakeTable::with_rows(vec![stored]);
    let request: UpdateMCPToolsetRequest =
        serde_json::from_str(r#"{"toolset_id": "id-1", "tools": []}"#).unwrap();
    let updated = update_mcp_toolset(&table, &request, "editor")
        .await
        .unwrap()
        .unwrap();
    assert!(updated.tools.is_empty());
    assert_eq!(table.rows.lock().unwrap()[0].tools.as_deref(), Some("[]"));
}

#[tokio::test]
async fn update_returns_none_when_the_toolset_is_not_found() {
    let table = FakeTable::with_rows(vec![toolset_row("id-1", "alpha")]);
    let request: UpdateMCPToolsetRequest =
        serde_json::from_str(r#"{"toolset_id": "id-9", "description": "changed"}"#).unwrap();
    assert!(
        update_mcp_toolset(&table, &request, "editor")
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn delete_returns_the_removed_toolset() {
    let table = FakeTable::with_rows(vec![toolset_row("id-1", "alpha")]);
    let removed = delete_mcp_toolset(&table, "id-1").await.unwrap();
    assert_eq!(
        removed.map(|toolset| toolset.toolset_name),
        Some("alpha".to_owned())
    );
    assert!(table.rows.lock().unwrap().is_empty());
}

#[tokio::test]
async fn delete_returns_none_when_the_toolset_is_not_found() {
    let table = FakeTable::with_rows(vec![toolset_row("id-1", "alpha")]);
    assert!(delete_mcp_toolset(&table, "id-2").await.unwrap().is_none());
}

#[test]
fn toolset_from_row_parses_the_stored_tools_json() {
    let mut row = toolset_row("id-1", "alpha");
    row.tools = Some(
        r#"[{"server_id":"s1","tool_name":"t1"},{"server_id":"s2","tool_name":"t2"}]"#.to_owned(),
    );
    let parsed = toolset_from_row(row).unwrap();
    assert_eq!(parsed.tools, vec![tool("s1", "t1"), tool("s2", "t2")]);
}

#[test]
fn toolset_from_row_reads_a_null_tools_column_as_an_empty_list() {
    let mut row = toolset_row("id-1", "alpha");
    row.tools = None;
    let parsed = toolset_from_row(row).unwrap();
    assert!(parsed.tools.is_empty());
}

#[test]
fn toolset_from_row_rejects_unreadable_tools_json() {
    let mut row = toolset_row("id-1", "alpha");
    row.tools = Some("not json".to_owned());
    let error = toolset_from_row(row).unwrap_err();
    assert!(matches!(error, MCPToolsetStorageError::Tools(_)));
}
