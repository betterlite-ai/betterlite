#![cfg(test)]

use std::convert::Infallible;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use betterlite_config::sections::database_startup::job::{BuildFuture, IndexBuild};
use betterlite_db::migration_drift::comparison::SchemaCheck;
use betterlite_db::migration_recovery::RecoveryError;
use betterlite_db::worker_heartbeat::{
    HeartbeatDatabase, HeartbeatFuture, HostnameError, ProxyWorkerHeartbeat, WorkerIdentity,
};
use tokio::sync::Notify;

use super::refresh::{CredentialRefresh, RefreshFuture};
use super::serving::{StartupDatabase, StartupFuture};

// The two error classes the startup policy distinguishes, produced without a live database.
#[derive(Debug, Clone, Copy)]
pub(crate) enum MigrationAnswer {
    Applied,
    Unverified,
    Exhausted,
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum BuildAnswer {
    Immediate(bool),
    WaitForRelease,
}

pub(crate) struct ScriptedStartup {
    pub(crate) check: SchemaCheck,
    pub(crate) migrate: MigrationAnswer,
    pub(crate) build: BuildAnswer,
    pub(crate) release: Arc<Notify>,
    pub(crate) finished: Arc<AtomicBool>,
    pub(crate) calls: Mutex<Vec<&'static str>>,
}

impl ScriptedStartup {
    pub(crate) fn new() -> Self {
        Self {
            check: SchemaCheck::InSync,
            migrate: MigrationAnswer::Applied,
            build: BuildAnswer::Immediate(true),
            release: Arc::new(Notify::new()),
            finished: Arc::new(AtomicBool::new(false)),
            calls: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn calls(&self) -> Vec<&'static str> {
        self.calls.lock().expect("calls").clone()
    }
}

impl StartupDatabase for ScriptedStartup {
    fn check(&self) -> StartupFuture<'_, SchemaCheck> {
        self.calls.lock().expect("calls").push("check");
        let check = self.check.clone();
        Box::pin(async move { check })
    }

    fn migrate(&self) -> StartupFuture<'_, Result<(), RecoveryError>> {
        self.calls.lock().expect("calls").push("migrate");
        let answer = match self.migrate {
            MigrationAnswer::Applied => Ok(()),
            MigrationAnswer::Unverified => Err(RecoveryError::UnverifiedCompletion {
                message: "the migration completion could not be verified".to_owned(),
            }),
            MigrationAnswer::Exhausted => Err(RecoveryError::AttemptsExhausted {
                message: "no attempts left".to_owned(),
            }),
        };
        Box::pin(async move { answer })
    }

    fn build_indexes(&self) -> StartupFuture<'_, bool> {
        self.calls.lock().expect("calls").push("build");
        match self.build {
            BuildAnswer::Immediate(built) => Box::pin(async move { built }),
            BuildAnswer::WaitForRelease => {
                let release = Arc::clone(&self.release);
                let finished = Arc::clone(&self.finished);
                Box::pin(async move {
                    release.notified().await;
                    finished.store(true, Ordering::SeqCst);
                    true
                })
            }
        }
    }
}

// The migration job's build port: it signals when it starts, then blocks until the test releases it.
pub(crate) struct JobBuild {
    pub(crate) started: Arc<Notify>,
    pub(crate) release: Arc<Notify>,
    pub(crate) finished: Arc<AtomicBool>,
    pub(crate) answer: bool,
    pub(crate) wait: bool,
}

impl JobBuild {
    pub(crate) fn immediate(answer: bool) -> Self {
        Self {
            started: Arc::new(Notify::new()),
            release: Arc::new(Notify::new()),
            finished: Arc::new(AtomicBool::new(false)),
            answer,
            wait: false,
        }
    }

    pub(crate) fn waiting() -> Self {
        Self {
            wait: true,
            ..Self::immediate(true)
        }
    }
}

impl IndexBuild for JobBuild {
    fn build(&self) -> BuildFuture<'_> {
        self.started.notify_one();
        let release = Arc::clone(&self.release);
        let finished = Arc::clone(&self.finished);
        let answer = self.answer;
        let wait = self.wait;
        Box::pin(async move {
            if wait {
                release.notified().await;
            }
            finished.store(true, Ordering::SeqCst);
            answer
        })
    }
}

#[derive(Default)]
pub(crate) struct ScriptedRefresh {
    pub(crate) started: AtomicBool,
    pub(crate) stopped: AtomicBool,
}

impl CredentialRefresh for ScriptedRefresh {
    fn start(&self) {
        self.started.store(true, Ordering::SeqCst);
    }

    fn stop(&self) -> RefreshFuture<'_> {
        self.stopped.store(true, Ordering::SeqCst);
        Box::pin(async { Ok(()) })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HeartbeatCall {
    Upsert,
    Prune,
    Deregister,
}

#[derive(Default)]
pub(crate) struct ScriptedHeartbeat {
    pub(crate) calls: Mutex<Vec<HeartbeatCall>>,
}

impl ScriptedHeartbeat {
    pub(crate) fn calls(&self) -> Vec<HeartbeatCall> {
        self.calls.lock().expect("calls").clone()
    }

    fn record(&self, call: HeartbeatCall) {
        self.calls.lock().expect("calls").push(call);
    }
}

impl HeartbeatDatabase for ScriptedHeartbeat {
    type Error = Infallible;

    fn upsert<'database>(
        &'database self,
        _worker_id: &'database str,
        _hostname: &'database str,
    ) -> HeartbeatFuture<'database, (), Self::Error> {
        self.record(HeartbeatCall::Upsert);
        Box::pin(async { Ok(()) })
    }

    fn prune(&self, _retention_seconds: f64) -> HeartbeatFuture<'_, (), Self::Error> {
        self.record(HeartbeatCall::Prune);
        Box::pin(async { Ok(()) })
    }

    fn deregister<'database>(
        &'database self,
        _worker_id: &'database str,
    ) -> HeartbeatFuture<'database, (), Self::Error> {
        self.record(HeartbeatCall::Deregister);
        Box::pin(async { Ok(()) })
    }

    fn count_live_workers(
        &self,
        _window_seconds: f64,
    ) -> HeartbeatFuture<'_, Vec<Option<i32>>, Self::Error> {
        Box::pin(async { Ok(Vec::new()) })
    }
}

struct FixedIdentity {
    hostname: &'static str,
}

impl WorkerIdentity for FixedIdentity {
    fn hostname(&self) -> Result<String, HostnameError> {
        Ok(self.hostname.to_owned())
    }

    fn generate_worker_id(&self) -> String {
        "generated-worker".to_owned()
    }
}

pub(crate) fn heartbeat(worker_id: &str) -> ProxyWorkerHeartbeat {
    ProxyWorkerHeartbeat::new(Some(worker_id), &FixedIdentity { hostname: "host-a" })
        .expect("the fixed identity provides a hostname")
}
