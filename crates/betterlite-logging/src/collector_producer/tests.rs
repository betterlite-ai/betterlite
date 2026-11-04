#![cfg(test)]

use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

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
