#![cfg(test)]

use std::str::FromStr as _;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use betterlite_auth::reservation_settlement::handle::ReservationHandle;
use betterlite_openai::chat::response::ChatResponse;
use betterlite_redis::request_batch::lock;
use rstest::rstest;
use rust_decimal::Decimal;

use super::{
    Billing, BillingError, ChatAnswer, ChatRunner, InternalAuthorizer, ReservationRelease,
    ReserveBudget, complete,
};
use crate::lens_billing::internal_request::{ClientIp, Incoming, InternalRequest};
use crate::lens_inference::request_body::{AnalysisBody, RequestIds};

type Log = Arc<Mutex<Vec<String>>>;

#[derive(Default)]
struct Plan {
    outcome: Outcome,
    billed: Option<&'static str>,
    reservation: Option<ReservationHandle>,
}

#[derive(Default)]
enum Outcome {
    #[default]
    Proceed,
    Budget,
    ReserveFailure,
    RunFailure,
}

struct Fake {
    plan: Plan,
    log: Log,
}

struct Ip;

impl ClientIp for Ip {
    fn get_mcp_client_ip(&self, _incoming: &Incoming) -> Option<String> {
        Some("203.0.113.7".to_owned())
    }
}

#[async_trait]
impl InternalAuthorizer for Fake {
    async fn authorize(
        &self,
        key_id: &str,
        request: &InternalRequest,
        data: &AnalysisBody<'_>,
    ) -> Result<(), BillingError> {
        let mut log = lock(&self.log);
        log.push(format!(
            "authorize {key_id} {} {}",
            request.path, data.model
        ));
        log.push(format!("client {:?}", request.client));
        drop(log);
        if let Some(reservation) = &self.plan.reservation {
            request.reservation.set(reservation.clone());
        }
        if matches!(self.plan.outcome, Outcome::Budget) {
            return Err(BillingError::BudgetExceeded);
        }
        Ok(())
    }
}

#[async_trait]
impl ChatRunner for Fake {
    async fn run(
        &self,
        request: &InternalRequest,
        _data: &AnalysisBody<'_>,
    ) -> Result<ChatAnswer, BillingError> {
        let privacy = lock(&request.privacy.0).message_logging_disabled;
        lock(&self.log).push(format!("run privacy={privacy}"));
        if matches!(self.plan.outcome, Outcome::RunFailure) {
            return Err(BillingError::Http {
                status: 502,
                detail: "upstream failed".to_owned(),
            });
        }
        Ok(ChatAnswer {
            response: response(),
            billed_cost: self.plan.billed.map(str::to_owned),
        })
    }
}

#[async_trait]
impl ReserveBudget for Fake {
    async fn reserve(&self) -> Result<(), BillingError> {
        lock(&self.log).push("reserve".to_owned());
        if matches!(self.plan.outcome, Outcome::ReserveFailure) {
            return Err(BillingError::Http {
                status: 409,
                detail: "Could not reserve analysis budget".to_owned(),
            });
        }
        Ok(())
    }
}

#[async_trait]
impl ReservationRelease for Fake {
    async fn release(&self, _reservation: &ReservationHandle) {
        lock(&self.log).push("release".to_owned());
    }
}

fn stamped_reservation() -> ReservationHandle {
    ReservationHandle::new(Decimal::ZERO, Vec::new(), Decimal::ZERO)
}

fn response() -> ChatResponse {
    let json = concat!(
        "{\"id\":\"x\",\"object\":\"chat.completion\",\"created\":1,\"model\":\"lens-model\",",
        "\"choices\":[],\"usage\":{\"prompt_tokens\":2,\"completion_tokens\":4,",
        "\"total_tokens\":6}}"
    );
    serde_json::from_str(json).unwrap()
}

const fn body() -> AnalysisBody<'static> {
    AnalysisBody::new(
        "lens-model",
        "hello",
        RequestIds {
            lens_id: "lens-1",
            run_id: "job-1",
            worker_id: "worker-1",
            team_id: Some("team-1"),
        },
    )
}

fn harness(plan: Plan) -> (Billing, Arc<Fake>, Log) {
    let log: Log = Log::default();
    let fake = Arc::new(Fake {
        plan,
        log: Arc::clone(&log),
    });
    // Arc::clone cannot widen to the fields' trait objects, so each is cloned as its own type
    let authorizer = Arc::clone(&fake);
    let runner = Arc::clone(&fake);
    let releaser = Arc::clone(&fake);
    let billing = Billing {
        authorizer,
        runner,
        releaser,
        client_ip: Arc::new(Ip),
    };
    (billing, fake, log)
}

fn lines(log: &Log) -> Vec<String> {
    lock(log).clone()
}

fn releases(log: &Log) -> usize {
    lines(log).iter().filter(|line| *line == "release").count()
}

#[tokio::test]
async fn complete_authorizes_reserves_then_runs() {
    let (billing, fake, log) = harness(Plan::default());
    let answer = complete(
        &billing,
        "key-1",
        &body(),
        fake.as_ref(),
        &Incoming::default(),
    )
    .await
    .unwrap();
    assert!(answer.billed_cost.is_none());
    assert_eq!(
        lines(&log),
        vec![
            "authorize key-1 /v1/chat/completions lens-model".to_owned(),
            "client Some((\"203.0.113.7\", 0))".to_owned(),
            "reserve".to_owned(),
            "run privacy=false".to_owned(),
        ]
    );
}

#[tokio::test]
async fn a_budget_error_from_the_authorizer_is_the_fixed_402() {
    let plan = Plan {
        outcome: Outcome::Budget,
        billed: None,
        reservation: Some(stamped_reservation()),
    };
    let (billing, fake, log) = harness(plan);
    let error = complete(
        &billing,
        "key-1",
        &body(),
        fake.as_ref(),
        &Incoming::default(),
    )
    .await
    .unwrap_err();
    assert_eq!(error.status(), 402);
    assert_eq!(
        error.detail(),
        "The analysis key or its owner has reached a budget limit"
    );
    assert_eq!(releases(&log), 1);
    assert!(!lines(&log).contains(&"reserve".to_owned()));
}

#[tokio::test]
async fn a_reserve_failure_releases_and_skips_the_chat_call() {
    let plan = Plan {
        outcome: Outcome::ReserveFailure,
        billed: None,
        reservation: Some(stamped_reservation()),
    };
    let (billing, fake, log) = harness(plan);
    let error = complete(
        &billing,
        "key-1",
        &body(),
        fake.as_ref(),
        &Incoming::default(),
    )
    .await
    .unwrap_err();
    assert_eq!(error.status(), 409);
    assert!(!lines(&log).iter().any(|line| line.starts_with("run")));
    assert_eq!(releases(&log), 1);
}

#[tokio::test]
async fn a_chat_failure_releases_the_reservation() {
    let plan = Plan {
        outcome: Outcome::RunFailure,
        billed: None,
        reservation: Some(stamped_reservation()),
    };
    let (billing, fake, log) = harness(plan);
    let error = complete(
        &billing,
        "key-1",
        &body(),
        fake.as_ref(),
        &Incoming::default(),
    )
    .await
    .unwrap_err();
    assert_eq!(error.status(), 502);
    assert_eq!(releases(&log), 1);
}

#[tokio::test]
async fn a_callback_bound_reservation_is_not_released() {
    let reservation = ReservationHandle::new(Decimal::ZERO, Vec::new(), Decimal::ZERO);
    lock(&reservation.0).callback_bound = true;
    let plan = Plan {
        outcome: Outcome::Proceed,
        billed: None,
        reservation: Some(reservation),
    };
    let (billing, fake, log) = harness(plan);
    complete(
        &billing,
        "key-1",
        &body(),
        fake.as_ref(),
        &Incoming::default(),
    )
    .await
    .unwrap();
    assert_eq!(releases(&log), 0);
}

#[tokio::test]
async fn an_unstamped_reservation_is_not_released() {
    let (billing, fake, log) = harness(Plan::default());
    complete(
        &billing,
        "key-1",
        &body(),
        fake.as_ref(),
        &Incoming::default(),
    )
    .await
    .unwrap();
    assert_eq!(releases(&log), 0);
}

#[rstest]
#[case(None, None)]
#[case(Some(""), None)]
#[case(Some("None"), None)]
#[case(Some("0"), Some("0"))]
#[case(Some("4.5"), Some("4.5"))]
#[tokio::test]
async fn the_billed_cost_header_is_parsed(
    #[case] billed: Option<&'static str>,
    #[case] expected: Option<&'static str>,
) {
    let plan = Plan {
        outcome: Outcome::Proceed,
        billed,
        reservation: None,
    };
    let (billing, fake, _log) = harness(plan);
    let answer = complete(
        &billing,
        "key-1",
        &body(),
        fake.as_ref(),
        &Incoming::default(),
    )
    .await
    .unwrap();
    let expected_cost = expected.map(|text| Decimal::from_str(text).unwrap());
    assert_eq!(answer.billed_cost, expected_cost);
}

#[tokio::test]
async fn an_unparsable_billed_cost_fails_the_call() {
    let plan = Plan {
        outcome: Outcome::Proceed,
        billed: Some("cheap"),
        reservation: Some(stamped_reservation()),
    };
    let (billing, fake, log) = harness(plan);
    let error = complete(
        &billing,
        "key-1",
        &body(),
        fake.as_ref(),
        &Incoming::default(),
    )
    .await
    .unwrap_err();
    assert_eq!(error.status(), 500);
    assert_eq!(releases(&log), 1);
}
