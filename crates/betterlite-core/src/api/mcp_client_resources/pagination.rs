use std::collections::BTreeSet;
use std::error::Error;
use std::time::Duration;

use betterlite_mcp::client::sessions::error::{McpFailure, SessionError};
use tokio::time::{Instant, timeout};
use tracing::warn;

use super::types::PageCursor;

pub(crate) const MAX_PAGES: usize = 1_000;
pub(crate) const METHOD_NOT_FOUND: i32 = -32_601;

#[derive(Debug)]
pub(crate) enum PageFailure<Source>
where
    Source: Error + Send + Sync + 'static,
{
    Session(Box<SessionError<Source>>),
    Unavailable(McpFailure),
}

pub(crate) async fn list_optional_pages<Source, Page, Item, Fetch, Items>(
    deadline: Duration,
    mut fetch_page: Fetch,
    items_of: Items,
) -> Result<Vec<Item>, PageFailure<Source>>
where
    Source: Error + Send + Sync + 'static,
    Page: PageCursor,
    Item: Clone,
    Fetch: AsyncFnMut(Option<String>) -> Result<Page, SessionError<Source>>,
    Items: Fn(&Page) -> &[Item],
{
    let started = Instant::now();
    let mut items = Vec::new();
    let mut cursors = BTreeSet::new();
    let mut cursor: Option<String> = None;
    for page_index in 0..MAX_PAGES {
        let remaining = deadline.saturating_sub(started.elapsed());
        if remaining.is_zero() {
            warn!("MCP client list pagination passed its deadline after {page_index} pages");
            return Ok(items);
        }
        let page = match timeout(remaining, fetch_page(cursor.clone())).await {
            Ok(Ok(page)) => page,
            Ok(Err(error)) => return Err(page_failure(error, page_index)),
            Err(_elapsed) => {
                warn!("MCP client list pagination passed its deadline after {page_index} pages");
                return Ok(items);
            }
        };
        items.extend_from_slice(items_of(&page));
        let Some(next_cursor) = page.next_page().map(str::to_owned) else {
            return Ok(items);
        };
        if !cursors.insert(next_cursor.clone()) {
            warn!("MCP client list pagination repeated a cursor after {page_index} pages");
            return Ok(items);
        }
        cursor = Some(next_cursor);
    }
    warn!("MCP client list pagination reached the {MAX_PAGES} page limit");
    Ok(items)
}

fn page_failure<Source>(error: SessionError<Source>, page_index: usize) -> PageFailure<Source>
where
    Source: Error + Send + Sync + 'static,
{
    if page_index > 0
        && let SessionError::Protocol(failure) = &error
        && failure.code == METHOD_NOT_FOUND
    {
        return PageFailure::Unavailable(failure.clone());
    }
    PageFailure::Session(Box::new(error))
}
