#![cfg(test)]

use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::io::AsyncReadExt;

fn fallback(counter: Arc<AtomicUsize>) -> Fallback {
    Arc::new(move |_| {
        let counter = Arc::clone(&counter);
        Box::pin(async move {
            counter.fetch_add(1, Ordering::SeqCst);
            Ok(())
        })
    })
}

#[tokio::test]
async fn falls_back_when_closed() {
    let counter = Arc::new(AtomicUsize::new(0));
    let producer = SpendEventProducer::new(
        CollectorAddress::Unix {
            path: "/missing".to_owned(),
        },
        1,
        0.1,
        UnavailablePolicy::Fallback,
        fallback(Arc::clone(&counter)),
    );
    producer.close(Duration::ZERO).await;
    assert_eq!(
        producer.publish(b"line".to_vec()).await,
        PublishOutcome::Fallback
    );
    assert_eq!(counter.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn drains_queued_lines_in_order_before_close() {
    let path =
        std::env::temp_dir().join(format!("betterlite-collector-{}.sock", std::process::id()));
    let _removed = std::fs::remove_file(&path);
    let listener = tokio::net::UnixListener::bind(&path).unwrap();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut received = vec![0; 12];
        stream.read_exact(&mut received).await.unwrap();
        received
    });
    let counter = Arc::new(AtomicUsize::new(0));
    let producer = SpendEventProducer::new(
        CollectorAddress::Unix {
            path: path.to_string_lossy().into_owned(),
        },
        4,
        1.0,
        UnavailablePolicy::Fallback,
        fallback(Arc::clone(&counter)),
    );
    assert_eq!(
        producer.publish(b"first\n".to_vec()).await,
        PublishOutcome::Queued
    );
    assert_eq!(
        producer.publish(b"other\n".to_vec()).await,
        PublishOutcome::Queued
    );
    producer.close(Duration::from_secs(2)).await;
    assert_eq!(server.await.unwrap(), b"first\nother\n");
    assert_eq!(counter.load(Ordering::SeqCst), 0);
    let _removed = std::fs::remove_file(path);
}

#[tokio::test]
async fn applies_fallback_to_queued_lines_after_drain_timeout() {
    let counter = Arc::new(AtomicUsize::new(0));
    let producer = SpendEventProducer::new(
        CollectorAddress::Unix {
            path: "/missing-collector.sock".to_owned(),
        },
        4,
        1.0,
        UnavailablePolicy::Fallback,
        fallback(Arc::clone(&counter)),
    );
    assert_eq!(
        producer.publish(b"first\n".to_vec()).await,
        PublishOutcome::Queued
    );
    assert_eq!(
        producer.publish(b"other\n".to_vec()).await,
        PublishOutcome::Queued
    );
    producer.close(Duration::ZERO).await;
    assert_eq!(counter.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn fallback_finishes_after_publish_is_cancelled() {
    let (started_tx, started_rx) = tokio::sync::oneshot::channel();
    let (continue_tx, continue_rx) = tokio::sync::oneshot::channel();
    let (finished_tx, finished_rx) = tokio::sync::oneshot::channel();
    let started_tx = Arc::new(tokio::sync::Mutex::new(Some(started_tx)));
    let continue_rx = Arc::new(tokio::sync::Mutex::new(Some(continue_rx)));
    let finished_tx = Arc::new(tokio::sync::Mutex::new(Some(finished_tx)));
    let fallback: Fallback = Arc::new(
        move |_| -> Pin<Box<dyn Future<Output = Result<(), FallbackError>> + Send>> {
            let started_tx = Arc::clone(&started_tx);
            let continue_rx = Arc::clone(&continue_rx);
            let finished_tx = Arc::clone(&finished_tx);
            Box::pin(async move {
                if let Some(started_tx) = started_tx.lock().await.take() {
                    let _sent = started_tx.send(());
                }
                if let Some(continue_rx) = continue_rx.lock().await.take() {
                    let _continued = continue_rx.await;
                }
                if let Some(finished_tx) = finished_tx.lock().await.take() {
                    let _sent = finished_tx.send(());
                }
                Ok(())
            })
        },
    );
    let producer = SpendEventProducer::new(
        CollectorAddress::Unix {
            path: "/missing-collector.sock".to_owned(),
        },
        1,
        1.0,
        UnavailablePolicy::Fallback,
        fallback,
    );
    producer.close(Duration::ZERO).await;
    let producer = Arc::new(producer);
    let publishing = {
        let producer = Arc::clone(&producer);
        tokio::spawn(async move { producer.publish(b"event\n".to_vec()).await })
    };
    let _started = started_rx.await;
    publishing.abort();
    let _continued = continue_tx.send(());
    let _finished = finished_rx.await;
}

#[tokio::test]
async fn applies_policy_to_full_queue_and_partially_written_line_on_close() {
    let path = std::env::temp_dir().join(format!(
        "betterlite-collector-full-{}.sock",
        std::process::id()
    ));
    let _removed = std::fs::remove_file(&path);
    let listener = tokio::net::UnixListener::bind(&path).unwrap();
    let (release_tx, release_rx) = tokio::sync::oneshot::channel();
    let (accepted_tx, accepted_rx) = tokio::sync::oneshot::channel();
    let accepted = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let _sent = accepted_tx.send(());
        let _release = release_rx.await;
        drop(stream);
    });
    let counter = Arc::new(AtomicUsize::new(0));
    let producer = SpendEventProducer::new(
        CollectorAddress::Unix {
            path: path.to_string_lossy().into_owned(),
        },
        1,
        1.0,
        UnavailablePolicy::Fallback,
        fallback(Arc::clone(&counter)),
    );
    assert_eq!(
        producer.publish(vec![b'x'; 32 * 1024 * 1024]).await,
        PublishOutcome::Queued
    );
    tokio::time::timeout(Duration::from_secs(1), accepted_rx)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        producer.publish(b"queued\n".to_vec()).await,
        PublishOutcome::Queued
    );
    assert_eq!(
        producer.publish(b"full\n".to_vec()).await,
        PublishOutcome::Fallback
    );
    producer.close(Duration::ZERO).await;
    assert_eq!(counter.load(Ordering::SeqCst), 3);
    let _released = release_tx.send(());
    let _accepted = accepted.await;
    let _removed = std::fs::remove_file(path);
}

#[tokio::test]
async fn applies_policy_after_peer_half_close() {
    let path = std::env::temp_dir().join(format!(
        "betterlite-collector-eof-{}.sock",
        std::process::id()
    ));
    let _removed = std::fs::remove_file(&path);
    let listener = tokio::net::UnixListener::bind(&path).unwrap();
    let (half_closed_tx, half_closed_rx) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        let mut stream = listener.accept().await.unwrap().0;
        let mut received = [0; 6];
        stream.read_exact(&mut received).await.unwrap();
        tokio::io::AsyncWriteExt::shutdown(&mut stream)
            .await
            .unwrap();
        let _sent = half_closed_tx.send(());
    });
    let counter = Arc::new(AtomicUsize::new(0));
    let producer = SpendEventProducer::new(
        CollectorAddress::Unix {
            path: path.to_string_lossy().into_owned(),
        },
        1,
        1.0,
        UnavailablePolicy::Fallback,
        fallback(Arc::clone(&counter)),
    );
    assert_eq!(
        producer.publish(b"event\n".to_vec()).await,
        PublishOutcome::Queued
    );
    let _half_closed = half_closed_rx.await;
    assert_eq!(
        producer.publish(b"next\n".to_vec()).await,
        PublishOutcome::Queued
    );
    producer.close(Duration::from_secs(1)).await;
    server.await.unwrap();
    assert_eq!(counter.load(Ordering::SeqCst), 1);
    let _removed = std::fs::remove_file(path);
}

#[tokio::test(start_paused = true)]
async fn reconnect_backoff_uses_injected_tokio_time() {
    let counter = Arc::new(AtomicUsize::new(0));
    let notify = Arc::new(tokio::sync::Notify::new());
    let callback_counter = Arc::clone(&counter);
    let callback_notify = Arc::clone(&notify);
    let fallback: Fallback = Arc::new(move |_| {
        let counter = Arc::clone(&callback_counter);
        let notify = Arc::clone(&callback_notify);
        Box::pin(async move {
            counter.fetch_add(1, Ordering::SeqCst);
            notify.notify_one();
            Ok(())
        })
    });
    let producer = SpendEventProducer::new(
        CollectorAddress::Unix {
            path: "/missing-collector.sock".to_owned(),
        },
        4,
        1.0,
        UnavailablePolicy::Fallback,
        fallback,
    );
    assert_eq!(
        producer.publish(b"first\n".to_vec()).await,
        PublishOutcome::Queued
    );
    notify.notified().await;
    assert_eq!(
        producer.publish(b"backoff\n".to_vec()).await,
        PublishOutcome::Fallback
    );
    tokio::time::advance(Duration::from_secs(1)).await;
    assert_eq!(
        producer.publish(b"after\n".to_vec()).await,
        PublishOutcome::Queued
    );
    notify.notified().await;
    producer.close(Duration::ZERO).await;
    assert_eq!(counter.load(Ordering::SeqCst), 3);
}
