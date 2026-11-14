use std::sync::Arc;

use jiff::SignedDuration;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::chirp_backend::commands::ChirpEvent;
use crate::chirp_stream::lifecycle::RecognizeStream;
use crate::chirp_stream::outbox::OutboxItem;

pub(super) enum Link {
    Event(String),
    Stream(Arc<RecognizeStream>),
    TurnDiscarded(usize),
}

pub(super) async fn run_pump(
    mut links: mpsc::Receiver<Link>,
    outbox: mpsc::Sender<OutboxItem>,
    shutdown: CancellationToken,
) -> Result<(), serde_json::Error> {
    let outcome = pump_links(&mut links, &outbox, &shutdown).await;
    while let Ok(link) = links.try_recv() {
        if let Link::Stream(stream) = link {
            stream.connection.close();
        }
    }
    outcome
}

async fn relay_stream(
    stream: &Arc<RecognizeStream>,
    outbox: &mpsc::Sender<OutboxItem>,
    shutdown: &CancellationToken,
    billed_before: f64,
) -> SignedDuration {
    let relayed = shutdown
        .run_until_cancelled(stream.relay(outbox.clone(), billed_before))
        .await;
    match relayed {
        Some(billed) => billed,
        None => {
            // cancelling the stream closes its connection before the relay reports
            stream.halt.cancel();
            stream.relay(outbox.clone(), billed_before).await
        }
    }
}

async fn pump_links(
    links: &mut mpsc::Receiver<Link>,
    outbox: &mpsc::Sender<OutboxItem>,
    shutdown: &CancellationToken,
) -> Result<(), serde_json::Error> {
    let mut billed_before = 0.0_f64;
    loop {
        let received = tokio::select! {
            biased;
            () = shutdown.cancelled() => break,
            received = links.recv() => received,
        };
        let Some(link) = received else {
            break;
        };
        match link {
            Link::Event(event) => {
                let _queued = shutdown
                    .run_until_cancelled(outbox.send(OutboxItem::Text(event)))
                    .await;
            }
            Link::Stream(stream) => {
                let billed = relay_stream(&stream, outbox, shutdown, billed_before).await;
                billed_before = billed_before.mul_add(1.0, billed.as_secs_f64());
            }
            Link::TurnDiscarded(turn) => {
                let event = serde_json::to_string(&ChirpEvent::TurnDiscarded {
                    billed_seconds: billed_before,
                })?;
                let _queued = shutdown
                    .run_until_cancelled(
                        outbox.send(OutboxItem::TurnDiscardedEvent { turn, event }),
                    )
                    .await;
            }
        }
    }
    Ok(())
}
