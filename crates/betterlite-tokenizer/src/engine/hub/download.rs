use super::classify::classify;
use super::{Clock, HubDownloader, HubError, Request};
use futures_util::future::{Either, select};
use hf_hub::progress::{DownloadEvent, Progress, ProgressEvent, ProgressHandler};
use hf_hub::{HFClient, split_id};
use std::future::Future;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tokio::sync::Notify;
use tokio::time::{Instant, timeout_at};
pub(super) const RETRY_START_TIMEOUT: Duration = Duration::from_secs(60);
const TOKENIZER_FILE: &str = "tokenizer.json";
#[derive(Debug, Clone, Copy)]
pub(super) struct Bounds {
    pub(super) start: Duration,
    pub(super) idle: Duration,
}
pub(super) async fn download_bounded(
    downloader: &HubDownloader,
    request: &Request<'_>,
    local_files_only: bool,
    bounds: Bounds,
) -> Result<PathBuf, HubError> {
    let client = downloader.client(request.token)?;
    let (owner, name) = split_id(request.repository);
    let events = Arc::new(DownloadEvents {
        notify: Notify::new(),
        started: AtomicBool::new(false),
    });
    let repo = client.model(owner.to_owned(), name.to_owned());
    let call = repo
        .download_file()
        .filename(TOKENIZER_FILE.to_owned())
        .revision(request.revision.to_owned())
        .local_files_only(local_files_only)
        .progress(Progress::from(Arc::clone(&events)))
        .send();
    let watch = Watch {
        events: &events,
        clock: downloader.clock.as_ref(),
        client: &client,
    };
    run_bounded(call, &watch, bounds, request).await
}
struct Watch<'watch> {
    events: &'watch DownloadEvents,
    clock: &'watch dyn Clock,
    client: &'watch HFClient,
}
struct DownloadEvents {
    notify: Notify,
    started: AtomicBool,
}
impl ProgressHandler for DownloadEvents {
    fn on_progress(&self, event: &ProgressEvent) {
        match event {
            ProgressEvent::Download(download) => match download {
                DownloadEvent::Start { .. } => {
                    self.started.store(true, Ordering::SeqCst);
                    self.notify.notify_one();
                }
                DownloadEvent::Progress { .. } | DownloadEvent::AggregateProgress { .. } => {
                    self.notify.notify_one();
                }
                DownloadEvent::Complete => {}
            },
            ProgressEvent::Upload(_) => {}
        }
    }
}
async fn run_bounded<C>(
    call: C,
    watch: &Watch<'_>,
    bounds: Bounds,
    request: &Request<'_>,
) -> Result<PathBuf, HubError>
where
    C: Future<Output = hf_hub::HFResult<PathBuf>>,
{
    let mut pinned = std::pin::pin!(call);
    let mut started = false;
    let mut deadline = watch
        .clock
        .now()
        .checked_add(bounds.start)
        .ok_or(HubError::TimeoutRange)?;
    loop {
        let signal = watch.events.notify.notified();
        tokio::pin!(signal);
        match timeout_at(deadline, select(pinned.as_mut(), signal.as_mut())).await {
            Err(_elapsed) if started => {
                return Err(HubError::Stalled(
                    request.repository.to_owned(),
                    bounds.idle.as_secs(),
                ));
            }
            Err(_elapsed) => return Err(HubError::StartTimeout(request.repository.to_owned())),
            Ok(Either::Left((result, _))) => {
                let failure = match result {
                    Ok(path) => return Ok(path),
                    Err(error) => error,
                };
                let failed = classify(watch.client, failure, request.repository, request.revision);
                return Err(failed.await);
            }
            Ok(Either::Right((_permit, _))) => {
                deadline = next_deadline(watch, &mut started, bounds)?;
            }
        }
    }
}
fn next_deadline(
    watch: &Watch<'_>,
    started: &mut bool,
    bounds: Bounds,
) -> Result<Instant, HubError> {
    let window = if watch.events.started.load(Ordering::SeqCst) {
        *started = true;
        bounds.idle
    } else {
        bounds.start
    };
    watch
        .clock
        .now()
        .checked_add(window)
        .ok_or(HubError::TimeoutRange)
}
