use std::future::{Future, pending};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use futures_util::future::{Either, select};
use hf_hub::HFError;
use hf_hub::progress::{DownloadEvent, Progress, ProgressEvent, ProgressHandler};
use hf_hub::split_id;
use tokio::sync::mpsc;
use tokio::time::timeout_at;

use super::classify::classify;
use super::{Clock, HubDownloader, HubError, Request};

pub(super) const RETRY_START_TIMEOUT: Duration = Duration::from_secs(60);
const EVENT_QUEUE: usize = 8;
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
    let repo = client.model(owner.to_owned(), name.to_owned());
    let (events, receiver) = DownloadEvents::new();
    let call = repo
        .download_file()
        .filename(TOKENIZER_FILE.to_owned())
        .revision(request.revision.to_owned())
        .local_files_only(local_files_only)
        .progress(Progress::from(Arc::clone(&events)))
        .send();
    run_bounded(call, &events, receiver, downloader.clock.as_ref(), bounds)
        .await
        .map_err(|failure| failure.classify(request.repository, request.revision))
}

struct DownloadEvents {
    sender: mpsc::Sender<()>,
    started: AtomicBool,
}

impl DownloadEvents {
    fn new() -> (Arc<Self>, mpsc::Receiver<()>) {
        let (sender, receiver) = mpsc::channel(EVENT_QUEUE);
        (
            Arc::new(Self {
                sender,
                started: AtomicBool::new(false),
            }),
            receiver,
        )
    }
}

impl ProgressHandler for DownloadEvents {
    fn on_progress(&self, event: &ProgressEvent) {
        match event {
            ProgressEvent::Download(download) => match download {
                DownloadEvent::Start { .. } => {
                    self.started.store(true, Ordering::SeqCst);
                    let _sent = self.sender.try_send(());
                }
                DownloadEvent::Progress { .. } | DownloadEvent::AggregateProgress { .. } => {
                    let _sent = self.sender.try_send(());
                }
                DownloadEvent::Complete => {}
            },
            ProgressEvent::Upload(_) => {}
        }
    }
}

enum Failure {
    Hub(HFError),
    StartTimeout,
    Stalled { seconds: u64 },
    TimeoutRange,
}

impl Failure {
    fn classify(self, repository: &str, revision: &str) -> HubError {
        match self {
            Self::Hub(error) => classify(error, repository, revision),
            Self::StartTimeout => HubError::StartTimeout {
                repository: repository.to_owned(),
            },
            Self::Stalled { seconds } => HubError::Stalled {
                repository: repository.to_owned(),
                seconds,
            },
            Self::TimeoutRange => HubError::TimeoutRange,
        }
    }
}

async fn run_bounded<C>(
    call: C,
    events: &DownloadEvents,
    mut receiver: mpsc::Receiver<()>,
    clock: &dyn Clock,
    bounds: Bounds,
) -> Result<PathBuf, Failure>
where
    C: Future<Output = hf_hub::HFResult<PathBuf>>,
{
    let mut pinned = std::pin::pin!(call);
    let mut started = false;
    let mut deadline = clock
        .now()
        .checked_add(bounds.start)
        .ok_or(Failure::TimeoutRange)?;
    loop {
        let signal = async {
            match receiver.recv().await {
                Some(()) => {}
                None => pending::<()>().await,
            }
        };
        tokio::pin!(signal);
        match timeout_at(deadline, select(pinned.as_mut(), signal.as_mut())).await {
            Err(_elapsed) => {
                if started {
                    return Err(Failure::Stalled {
                        seconds: bounds.idle.as_secs(),
                    });
                }
                return Err(Failure::StartTimeout);
            }
            Ok(Either::Left((result, _))) => return result.map_err(Failure::Hub),
            Ok(Either::Right(((), _))) => {
                let window = if events.started.load(Ordering::SeqCst) {
                    started = true;
                    bounds.idle
                } else {
                    bounds.start
                };
                deadline = clock
                    .now()
                    .checked_add(window)
                    .ok_or(Failure::TimeoutRange)?;
            }
        }
    }
}
