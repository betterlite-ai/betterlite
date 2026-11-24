use std::collections::VecDeque;
use std::future::{Future, ready};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_models::catalog::{BundledCatalog, CatalogError};
use betterlite_models::fetch_policy::client::{
    CatalogClient, CatalogClientError, CatalogRequest, FetchErrorKind,
};
use betterlite_models::fetch_policy::response::FetchResponse;
use betterlite_models::fetch_policy::retry::{CatalogSleeper, JitterSource};
use betterlite_models::reload::{
    CatalogService, FetchLimits, NoRuntimeRegistrations, ReloadOutcome,
};
use betterlite_models::source::{CatalogSource, CatalogStore};
use rust_decimal::Decimal;

use crate::periodic_reloads::sources::{
    CatalogServiceSource, JitterFactory, ModelCostMapSource as _,
};

use super::support::moment;

const URL: &str = "https://catalog.test/model.json";

type TestService = CatalogService<fn() -> Result<Vec<u8>, CatalogError>>;

#[derive(Debug, thiserror::Error)]
#[error("{detail}")]
struct TransportError {
    detail: String,
}

impl CatalogClientError for TransportError {
    fn kind(&self) -> FetchErrorKind {
        FetchErrorKind::Transport
    }
}

struct ScriptedClient {
    replies: Mutex<VecDeque<Result<FetchResponse, TransportError>>>,
    calls: AtomicUsize,
}

impl ScriptedClient {
    fn ok() -> Self {
        Self::new(Ok(FetchResponse::new(
            200,
            Vec::new(),
            catalog_body(60).into_bytes(),
        )))
    }

    fn unreachable(detail: &str) -> Self {
        Self::new(Err(TransportError {
            detail: detail.to_owned(),
        }))
    }

    fn new(reply: Result<FetchResponse, TransportError>) -> Self {
        Self {
            replies: Mutex::new(VecDeque::from([reply])),
            calls: AtomicUsize::new(0),
        }
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }

    fn next(&self) -> Result<FetchResponse, TransportError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .unwrap_or_else(|| {
                Err(TransportError {
                    detail: "unscripted request".to_owned(),
                })
            })
    }
}

impl CatalogClient for ScriptedClient {
    type Error = TransportError;

    fn get(&self, _request: &CatalogRequest) -> Result<FetchResponse, TransportError> {
        self.next()
    }

    fn get_async(
        &self,
        _request: &CatalogRequest,
    ) -> impl Future<Output = Result<FetchResponse, TransportError>> + Send {
        ready(self.next())
    }
}

struct NoSleeper;

impl CatalogSleeper for NoSleeper {
    fn sleep(&self, _seconds: Decimal) -> impl Future<Output = ()> + Send {
        ready(())
    }
}

struct ZeroJitter;

impl JitterSource for ZeroJitter {
    fn unit_interval(&mut self) -> Decimal {
        Decimal::ZERO
    }
}

struct ZeroJitters;

impl JitterFactory for ZeroJitters {
    type Jitter = ZeroJitter;

    fn create(&self) -> ZeroJitter {
        ZeroJitter
    }
}

fn catalog_body(count: usize) -> String {
    let entries = (0..count)
        .map(|index| format!("\"test-{index:02}\":{{\"litellm_provider\":\"openai\"}}"))
        .collect::<Vec<_>>()
        .join(",");
    format!("{{{entries}}}")
}

fn service() -> Arc<TestService> {
    let asset: fn() -> Result<Vec<u8>, CatalogError> = || Ok(catalog_body(60).into_bytes());
    Arc::new(CatalogService {
        source: CatalogSource::Remote {
            url: URL.to_owned(),
        },
        bundled: Mutex::new(BundledCatalog {
            asset,
            cached: None,
        }),
        clock: Arc::new(|| moment(1)),
        registrations: Arc::new(NoRuntimeRegistrations),
        current: CatalogStore::default(),
    })
}

fn limits() -> FetchLimits {
    FetchLimits {
        timeout: Duration::from_secs(5),
        max_attempts: 1_u32,
    }
}

#[tokio::test]
async fn a_successful_fetch_returns_the_reloaded_catalog() {
    let source = CatalogServiceSource {
        service: service(),
        limits: limits(),
        client: Arc::new(ScriptedClient::ok()),
        sleeper: Arc::new(NoSleeper),
        jitter: ZeroJitters,
    };
    let ReloadOutcome::Reloaded(snapshot) = source.refetch_cost_map().await else {
        panic!("a scripted 200 should reload the catalog");
    };
    assert_eq!(snapshot.catalog.adopted_size, 60);
}

#[tokio::test]
async fn an_unreachable_fetch_returns_unavailable() {
    let client = Arc::new(ScriptedClient::unreachable("the connection was refused"));
    let source = CatalogServiceSource {
        service: service(),
        limits: limits(),
        client: Arc::clone(&client),
        sleeper: Arc::new(NoSleeper),
        jitter: ZeroJitters,
    };
    let ReloadOutcome::Unavailable(unavailable) = source.refetch_cost_map().await else {
        panic!("a scripted transport error should leave the cost map unavailable");
    };
    assert_eq!(client.calls(), 1);
    assert!(unavailable.reason.contains("the connection was refused"));
}
