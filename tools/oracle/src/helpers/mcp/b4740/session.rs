use betterlite_mcp::tools::pagination::{McpSession, Page};
use betterlite_mcp::tools::schemas::McpTool;
use betterlite_mcp::tools::tool_calls::CallRequest;
use betterlite_prompts::arguments::json::Object;
use serde::Deserialize;
use std::collections::VecDeque;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use std::time::Duration;
use tracing_subscriber::{Layer, layer::Context};

#[derive(Clone, Debug, Deserialize)]
pub(crate) struct Frame {
    #[serde(default)]
    delay_ms: u64,
    #[serde(flatten)]
    page: Page,
}

#[derive(Default)]
pub(super) struct Fixture {
    pages: VecDeque<Frame>,
    pub(super) cursors: Vec<Option<String>>,
}

impl Fixture {
    pub(super) fn from_pages(pages: Vec<Frame>, generated: usize) -> Self {
        let mut frames = VecDeque::from(pages);
        frames.extend((0..generated).map(|index| Frame {
            delay_ms: 0,
            page: Page {
                tools: vec![McpTool {
                    name: "generated".to_owned(),
                    description: None,
                    input_schema: Some(Object::new()),
                    extra: Object::new(),
                }],
                next_cursor: Some(index.to_string()),
            },
        }));
        Self {
            pages: frames,
            cursors: Vec::new(),
        }
    }
}

impl McpSession for Fixture {
    type Error = std::io::Error;
    type Output = CallRequest;
    async fn list_tools(&mut self, cursor: Option<&str>) -> Result<Page, Self::Error> {
        self.cursors.push(cursor.map(str::to_owned));
        let frame = self
            .pages
            .pop_front()
            .ok_or_else(|| std::io::Error::other("missing page"))?;
        tokio::time::sleep(Duration::from_millis(frame.delay_ms)).await;
        Ok(frame.page)
    }
    fn call_tool(
        &mut self,
        request: CallRequest,
    ) -> impl Future<Output = Result<Self::Output, Self::Error>> + Send {
        std::future::ready(Ok(request))
    }
}

#[derive(Default)]
pub(super) struct Warnings(pub(super) Arc<AtomicUsize>);
impl<Current: tracing::Subscriber> Layer<Current> for Warnings {
    fn on_event(&self, event: &tracing::Event<'_>, _context: Context<'_, Current>) {
        if *event.metadata().level() == tracing::Level::WARN {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
    }
}
