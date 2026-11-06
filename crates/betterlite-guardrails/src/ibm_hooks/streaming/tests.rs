#![cfg(test)]

use std::time::Duration;

use futures_util::future::join;
use tokio::sync::mpsc;
use tokio::time::timeout;

use super::async_streaming_iterator_hook;

#[tokio::test]
async fn streaming_yields_every_item_unchanged_and_in_order() {
    let (sender, response) = mpsc::channel(4);
    let (output, mut relayed) = mpsc::channel(4);
    let hook = async_streaming_iterator_hook(response, output);
    let feed = async move {
        sender.send("one").await.expect("send");
        sender.send("two").await.expect("send");
        sender.send("three").await.expect("send");
    };

    let ((), ()) = tokio::join!(hook, feed);

    let mut items = Vec::new();
    while let Some(item) = relayed.recv().await {
        items.push(item);
    }
    assert_eq!(items, ["one", "two", "three"]);
}

#[tokio::test]
async fn streaming_stops_when_the_consumer_is_gone() {
    let (sender, response) = mpsc::channel(1);
    let (output, relayed) = mpsc::channel(1);
    drop(relayed);
    let hook = async_streaming_iterator_hook(response, output);
    let feed = async move {
        sender
            .send("one")
            .await
            .expect("the first send is buffered");
    };

    timeout(Duration::from_secs(1), join(hook, feed))
        .await
        .expect("the hook returns once the consumer is gone");
}
