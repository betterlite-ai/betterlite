use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use secrecy::SecretString;
use tokio::sync::Notify;
use tokio::task::{JoinSet, yield_now};
use tokio::time::timeout;

use crate::database_refresh::endpoint::DatabaseUrlSlot;
use crate::database_refresh::generation::GenerationOptions;
use crate::database_refresh::handle::PoolProvider as _;
use crate::database_refresh_loop::clock::RefreshClock;
use crate::database_refresh_loop::endpoint::WriterEndpoint;
use crate::database_refresh_loop::lifecycle::{DatabaseRefreshLoop, LoopTask, RefreshLoopConfig};
use crate::database_refresh_loop::minting::TokenMinter;
use crate::database_tokens::endpoint::IAMEndpoint;
use crate::database_tokens::expiry::TokenAuthKind;

use super::fakes::{FakeClock, FakeError, FakeHandle, FakeProvider, FakeScript, lock, secret, utc};

pub(crate) fn fixed_clock() -> Arc<dyn RefreshClock> {
    Arc::new(FakeClock {
        now: Mutex::new(utc("2024-01-01T00:00:00")),
    })
}

pub(crate) fn options(slot: DatabaseUrlSlot) -> GenerationOptions {
    GenerationOptions {
        iam_token_db_auth: false,
        token_auth: Some(TokenAuthKind::Rds),
        db_url_env_var: "DATABASE_URL".to_owned(),
        url: slot,
        iam_endpoint: None,
        recreate_uses_datasource: false,
        log_prefix: "[writer]".to_owned(),
    }
}

#[derive(Clone)]
pub(crate) enum MintOutcome {
    Token(String),
    Missing,
    Fail,
}

pub(crate) struct MintScript {
    pub(crate) outcomes: Mutex<VecDeque<MintOutcome>>,
    pub(crate) default_outcome: MintOutcome,
    pub(crate) endpoints: Mutex<Vec<IAMEndpoint>>,
}

impl MintScript {
    pub(crate) fn endpoint_count(&self) -> usize {
        lock(&self.endpoints).len()
    }

    pub(crate) fn last_endpoint(&self) -> Option<IAMEndpoint> {
        lock(&self.endpoints).last().cloned()
    }
}

pub(crate) struct FakeMinter {
    pub(crate) script: Arc<MintScript>,
}

impl TokenMinter for FakeMinter {
    type Error = FakeError;

    fn token(&self, endpoint: &IAMEndpoint) -> Result<Option<SecretString>, FakeError> {
        lock(&self.script.endpoints).push(endpoint.clone());
        let scripted = lock(&self.script.outcomes).pop_front();
        let outcome = scripted.unwrap_or_else(|| self.script.default_outcome.clone());
        match outcome {
            MintOutcome::Token(token) => Ok(Some(secret(&token))),
            MintOutcome::Missing => Ok(None),
            MintOutcome::Fail => Err(FakeError::Mint),
        }
    }
}

pub(crate) type FakeLoop = DatabaseRefreshLoop<FakeProvider, FakeMinter, FakeError>;
pub(crate) type FakeCallback = Arc<dyn Fn() -> Result<(), FakeError> + Send + Sync>;

pub(crate) struct LoopSpec {
    pub(crate) token_auth: Option<TokenAuthKind>,
    pub(crate) log_prefix: String,
    pub(crate) iam_endpoint: Option<IAMEndpoint>,
    pub(crate) initial_url: Option<String>,
    pub(crate) mint: MintOutcome,
    pub(crate) on_replaced: Option<FakeCallback>,
}

struct Wiring {
    script: Arc<FakeScript>,
    mint: Arc<MintScript>,
    clock: Arc<dyn RefreshClock>,
    slot: DatabaseUrlSlot,
    provider: Arc<FakeProvider>,
    initial: Arc<FakeHandle>,
    writer_endpoint: WriterEndpoint,
}

fn wiring(spec: &LoopSpec) -> Wiring {
    let script = Arc::new(FakeScript::default());
    let mint = Arc::new(MintScript {
        outcomes: Mutex::new(VecDeque::new()),
        default_outcome: spec.mint.clone(),
        endpoints: Mutex::new(Vec::new()),
    });
    let clock: Arc<dyn RefreshClock> = Arc::new(FakeClock {
        now: Mutex::new(utc("2024-01-01T00:00:00")),
    });
    let slot = DatabaseUrlSlot::default();
    drop(slot.publish(spec.initial_url.as_deref().map(secret)));
    let provider = Arc::new(FakeProvider {
        script: Arc::clone(&script),
    });
    let initial_url = spec.initial_url.as_deref().unwrap_or("postgres://initial");
    let initial = Arc::new(
        provider
            .build(&secret(initial_url), None)
            .expect("the initial handle builds"),
    );
    let writer_endpoint = WriterEndpoint {
        host: Some("db.example".to_owned()),
        port: "5432".to_owned(),
        user: Some("user".to_owned()),
        name: Some("litellm".to_owned()),
        schema: None,
    };
    Wiring {
        script,
        mint,
        clock,
        slot,
        provider,
        initial,
        writer_endpoint,
    }
}

pub(crate) struct LoopSetup {
    pub(crate) script: Arc<FakeScript>,
    pub(crate) mint: Arc<MintScript>,
    pub(crate) slot: DatabaseUrlSlot,
    pub(crate) initial: Arc<FakeHandle>,
    pub(crate) built: FakeLoop,
}

pub(crate) async fn build_loop(spec: LoopSpec) -> LoopSetup {
    let wiring = wiring(&spec);
    let options = GenerationOptions {
        iam_token_db_auth: false,
        token_auth: spec.token_auth,
        db_url_env_var: "DATABASE_URL".to_owned(),
        url: wiring.slot.clone(),
        iam_endpoint: spec.iam_endpoint,
        recreate_uses_datasource: false,
        log_prefix: spec.log_prefix,
    };
    let built = FakeLoop::new(
        Arc::clone(&wiring.provider),
        options,
        Some(Arc::clone(&wiring.initial)),
        RefreshLoopConfig {
            writer_endpoint: wiring.writer_endpoint,
            minter: FakeMinter {
                script: Arc::clone(&wiring.mint),
            },
            clock: Arc::clone(&wiring.clock),
        },
        spec.on_replaced,
    )
    .expect("the refresh loop builds");
    built
        .generation
        .connect(None)
        .await
        .expect("the initial handle connects");
    LoopSetup {
        script: wiring.script,
        mint: wiring.mint,
        slot: wiring.slot,
        initial: wiring.initial,
        built,
    }
}

pub(crate) async fn install_finished_task(built: &FakeLoop) -> Arc<Notify> {
    let mut tasks = JoinSet::new();
    tasks.spawn(async {});
    while tasks.join_next().await.is_some() {}
    let finished = Arc::new(Notify::new());
    let mut installed = lock(&built.task);
    *installed = Some(LoopTask {
        stop: Arc::clone(&finished),
        tasks,
    });
    drop(installed);
    finished
}

pub(crate) async fn wait_for_event(script: &FakeScript, wanted: &str) {
    let reached = timeout(Duration::from_secs(5), async {
        while !script.has_event(wanted) {
            yield_now().await;
        }
    })
    .await;
    reached.expect("the scripted event happens");
}
