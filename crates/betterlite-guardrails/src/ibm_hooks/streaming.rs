use tokio::sync::mpsc;

mod tests;

// LiteLLM's streaming hook is an async generator that yields the upstream items; a bounded relay keeps that shape.
pub async fn async_streaming_iterator_hook<Item>(
    mut response: mpsc::Receiver<Item>,
    output: mpsc::Sender<Item>,
) {
    while let Some(item) = response.recv().await {
        if output.send(item).await.is_err() {
            return;
        }
    }
}
