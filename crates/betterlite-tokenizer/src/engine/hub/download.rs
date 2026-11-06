use super::classify::classify;
use super::{Clock, HubDownloader, HubError, Request};
use futures_util::future::{Either, select};
use hf_hub::progress::{DownloadEvent, Progress, ProgressEvent, ProgressHandler};
use hf_hub::split_id;
use std::future::Future;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tokio::sync::Notify;
use tokio::time::timeout_at;
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
    run_bounded(call, &events, downloader.clock.as_ref(), bounds, request).await
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
    events: &DownloadEvents,
    clock: &dyn Clock,
    bounds: Bounds,
    request: &Request<'_>,
) -> Result<PathBuf, HubError>
where
    C: Future<Output = hf_hub::HFResult<PathBuf>>,
{
    let mut pinned = std::pin::pin!(call);
    let mut started = false;
    let mut deadline = clock
        .now()
        .checked_add(bounds.start)
        .ok_or(HubError::TimeoutRange)?;
    loop {
        let signal = events.notify.notified();
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
                return result
                    .map_err(|error| classify(error, request.repository, request.revision));
            }
            Ok(Either::Right((_permit, _))) => {
                let window = if events.started.load(Ordering::SeqCst) {
                    started = true;
                    bounds.idle
                } else {
                    bounds.start
                };
                deadline = clock
                    .now()
                    .checked_add(window)
                    .ok_or(HubError::TimeoutRange)?;
            }
        }
    }
}
