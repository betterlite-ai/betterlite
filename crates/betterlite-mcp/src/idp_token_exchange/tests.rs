#![cfg(test)]

use std::fmt::Debug;
use std::future::Future;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_db::database_faults::{DbError, ErrorType};
use rstest::rstest;
use tracing::field::{Field, Visit};
use tracing::subscriber::set_default;
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};
use tracing_subscriber::registry;

use super::{
    AuthorizeSubjectToken, AuthorizedSubject, DenialSource, EXCHANGE_ROUTE, IdpTokenExchange,
    REJECTED_SUBJECT_TOKEN, SUBJECT_CHECK_FAULTED, SUBJECT_CHECK_UNAVAILABLE, SubjectTokenDenial,
    TokenExchangeInputs, TokenExchangePrerequisites,
};
use crate::gateway_dcr::subject_token::{
    ExchangeSubjectToken, RefusalCode, SubjectIdentity, SubjectTokenRefusal,
};

const JWT: &str = "header.payload.signature";
const UNAVAILABLE_REASON: &str = "the identity provider timed out";

struct FixedInputs {
    prerequisites: TokenExchangePrerequisites,
}

impl TokenExchangeInputs for FixedInputs {
    fn prerequisites(&self) -> TokenExchangePrerequisites {
        self.prerequisites
    }
}

struct CountingInputs {
    prerequisites: TokenExchangePrerequisites,
    reads: Arc<AtomicUsize>,
}

impl TokenExchangeInputs for CountingInputs {
    fn prerequisites(&self) -> TokenExchangePrerequisites {
        self.reads.fetch_add(1, Ordering::SeqCst);
        self.prerequisites
    }
}

struct FakeAuthorizer {
    answer: Result<AuthorizedSubject, SubjectTokenDenial>,
    seen: Arc<Mutex<Vec<String>>>,
}

impl AuthorizeSubjectToken for FakeAuthorizer {
    type Context = ();

    fn authorize(
        &self,
        subject_token: &str,
        _context: &(),
    ) -> impl Future<Output = Result<AuthorizedSubject, SubjectTokenDenial>> + Send {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(subject_token.to_owned());
        let answer = self.answer.clone();
        async move { answer }
    }
}

struct Outcome {
    result: Result<SubjectIdentity, SubjectTokenRefusal>,
    logs: Vec<(String, String)>,
    seen: Vec<String>,
}

#[derive(Clone, Default)]
struct Logs(Arc<Mutex<Vec<(String, String)>>>);

impl Logs {
    fn entries(&self) -> Vec<(String, String)> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

struct Message(String);

impl Visit for Message {
    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?}");
        }
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            value.clone_into(&mut self.0);
        }
    }
}

impl<Current> Layer<Current> for Logs
where
    Current: Subscriber,
{
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, Current>) {
        let level = *event.metadata().level();
        let name = if level == Level::WARN {
            "WARNING"
        } else if level == Level::ERROR {
            "ERROR"
        } else {
            return;
        };
        let mut message = Message(String::new());
        event.record(&mut message);
        if !message.0.starts_with("token exchange ") {
            return;
        }
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((name.to_owned(), message.0));
    }
}

fn prerequisites() -> TokenExchangePrerequisites {
    TokenExchangePrerequisites {
        jwt_auth_enabled: true,
        has_database: true,
        maps_jwts_to_virtual_keys: false,
    }
}

fn subject(
    user_id: Option<&str>,
    team_id: Option<&str>,
    agent_id: Option<&str>,
) -> AuthorizedSubject {
    AuthorizedSubject {
        user_id: user_id.map(str::to_owned),
        team_id: team_id.map(str::to_owned),
        agent_id: agent_id.map(str::to_owned),
    }
}

fn denial(
    source: DenialSource,
    reason: &str,
    database_cause: Option<DbError>,
) -> SubjectTokenDenial {
    SubjectTokenDenial {
        source,
        reason: reason.to_owned(),
        database_cause,
    }
}

fn fault(error_type: ErrorType, message: &str) -> DbError {
    DbError {
        error_type,
        message: message.to_owned(),
        code: None,
        meta_code: None,
        traceback_modules: Vec::new(),
        cause: None,
    }
}

fn with_cause(mut error: DbError, cause: DbError) -> DbError {
    error.cause = Some(Box::new(cause));
    error
}

async fn run_with(
    subject_token: &str,
    expected: TokenExchangePrerequisites,
    answer: Result<AuthorizedSubject, SubjectTokenDenial>,
) -> Outcome {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let logs = Logs::default();
    let exchange = IdpTokenExchange {
        inputs: FixedInputs {
            prerequisites: expected,
        },
        authorizer: FakeAuthorizer {
            answer,
            seen: Arc::clone(&seen),
        },
    };
    let _guard = set_default(registry().with(logs.clone()));
    let result = exchange.exchange(subject_token, &()).await;
    Outcome {
        result,
        logs: logs.entries(),
        seen: seen.lock().unwrap_or_else(PoisonError::into_inner).clone(),
    }
}

#[test]
fn the_exchange_constants_match_litellms_token_route() {
    assert_eq!(EXCHANGE_ROUTE, "/token");
    assert_eq!(
        REJECTED_SUBJECT_TOKEN,
        "subject_token was rejected by the gateway's JWT auth"
    );
    assert_eq!(
        SUBJECT_CHECK_UNAVAILABLE,
        "the gateway could not verify subject_token because its identity provider or database is unavailable; retry"
    );
    assert_eq!(
        SUBJECT_CHECK_FAULTED,
        "the gateway could not verify subject_token because its database reported a fault that is not a transient outage; retrying will not help until the gateway deployment is repaired"
    );
}

#[rstest]
#[case::jwt_off(
    TokenExchangePrerequisites {
        jwt_auth_enabled: false,
        has_database: true,
        maps_jwts_to_virtual_keys: false,
    },
    "JWT auth is not enabled on this gateway, so it cannot exchange IdP tokens"
)]
#[case::no_database(
    TokenExchangePrerequisites {
        jwt_auth_enabled: true,
        has_database: false,
        maps_jwts_to_virtual_keys: true,
    },
    "this gateway has no database, so it cannot exchange IdP tokens"
)]
#[case::mapped(
    TokenExchangePrerequisites {
        jwt_auth_enabled: true,
        has_database: true,
        maps_jwts_to_virtual_keys: true,
    },
    "this gateway maps IdP tokens to virtual keys, which the exchange does not serve"
)]
fn refusal_names_the_first_unmet_prerequisite(
    #[case] expected: TokenExchangePrerequisites,
    #[case] description: &str,
) {
    let refusal = expected.refusal().unwrap();
    assert_eq!(refusal.error, RefusalCode::UnsupportedGrantType);
    assert_eq!(refusal.description, description);
}

#[test]
fn refusal_is_none_when_every_prerequisite_holds() {
    assert_eq!(prerequisites().refusal(), None);
}

#[tokio::test]
async fn a_prerequisite_refusal_comes_before_the_shape_test() {
    let unmet = TokenExchangePrerequisites {
        jwt_auth_enabled: false,
        has_database: false,
        maps_jwts_to_virtual_keys: false,
    };
    let outcome = run_with("garbage", unmet, Ok(subject(Some("user-1"), None, None))).await;
    let refusal = outcome.result.unwrap_err();
    assert_eq!(refusal.error, RefusalCode::UnsupportedGrantType);
    assert_eq!(
        refusal.description,
        "JWT auth is not enabled on this gateway, so it cannot exchange IdP tokens"
    );
    assert!(outcome.seen.is_empty());
    assert!(outcome.logs.is_empty());
}

#[rstest]
#[case::one_segment("garbage")]
#[case::two_segments("header.payload")]
#[case::four_segments("a.b.c.d")]
#[case::empty("")]
#[tokio::test]
async fn a_subject_token_that_is_not_a_jwt_is_refused_before_authorize(
    #[case] subject_token: &str,
) {
    let outcome = run_with(
        subject_token,
        prerequisites(),
        Ok(subject(Some("user-1"), None, None)),
    )
    .await;
    let refusal = outcome.result.unwrap_err();
    assert_eq!(refusal.error, RefusalCode::InvalidRequest);
    assert_eq!(refusal.description, "subject_token is not a JWT");
    assert!(outcome.seen.is_empty());
    assert!(outcome.logs.is_empty());
}

#[rstest]
#[case::with_team(Some("team-1"))]
#[case::without_team(None)]
#[tokio::test]
async fn an_identity_carries_the_users_team_or_none(#[case] team_id: Option<&str>) {
    let outcome = run_with(
        JWT,
        prerequisites(),
        Ok(subject(Some("user-1"), team_id, None)),
    )
    .await;
    assert_eq!(
        outcome.result.unwrap(),
        SubjectIdentity {
            user_id: "user-1".to_owned(),
            team_id: team_id.map(str::to_owned),
        }
    );
    assert_eq!(outcome.seen, vec![JWT.to_owned()]);
    assert!(outcome.logs.is_empty());
}

#[rstest]
#[case::agent_only(
    subject(None, None, Some("agent-1")),
    "Agent tokens require direct JWT authentication; this exchange supports users only"
)]
#[case::agent_before_user(
    subject(Some("user-1"), Some("team-1"), Some("agent-1")),
    "Agent tokens require direct JWT authentication; this exchange supports users only"
)]
#[case::no_user(
    subject(None, Some("team-1"), None),
    "subject_token names no user the gateway knows"
)]
#[tokio::test]
async fn an_agent_token_or_a_token_without_a_user_is_refused(
    #[case] answer: AuthorizedSubject,
    #[case] description: &str,
) {
    let outcome = run_with(JWT, prerequisites(), Ok(answer)).await;
    let refusal = outcome.result.unwrap_err();
    assert_eq!(refusal.error, RefusalCode::InvalidRequest);
    assert_eq!(refusal.description, description);
    assert!(outcome.logs.is_empty());
}

#[tokio::test]
async fn empty_user_id_is_no_user() {
    let outcome = run_with(
        JWT,
        prerequisites(),
        Ok(subject(Some(""), Some("team-1"), None)),
    )
    .await;
    let refusal = outcome.result.unwrap_err();
    let without_user = run_with(
        JWT,
        prerequisites(),
        Ok(subject(None, Some("team-1"), None)),
    )
    .await;
    assert_eq!(refusal.error, RefusalCode::InvalidRequest);
    assert_eq!(
        refusal.description,
        "subject_token names no user the gateway knows"
    );
    assert_eq!(refusal, without_user.result.unwrap_err());
    assert!(outcome.logs.is_empty());
}

#[rstest]
#[case::plain(DenialSource::Plain, "the signature did not verify")]
#[case::http_400(DenialSource::Http { status: 400 }, "bad subject token")]
#[case::http_401(
    DenialSource::Http { status: 401 },
    "the JWKS at https://idp.example/jwks.json rejected the token"
)]
#[case::http_499(DenialSource::Http { status: 499 }, "the client went away")]
#[case::proxy_429(
    DenialSource::Proxy { code: "429".to_owned() },
    "too many requests"
)]
#[case::proxy_words(
    DenialSource::Proxy { code: "abc".to_owned() },
    "the code is not a number"
)]
#[case::proxy_5xx(
    DenialSource::Proxy { code: "5xx".to_owned() },
    "still not a number"
)]
#[case::proxy_empty(
    DenialSource::Proxy { code: String::new() },
    "the code is empty"
)]
#[case::proxy_negative(
    DenialSource::Proxy { code: "-500".to_owned() },
    "a signed code"
)]
#[case::proxy_unicode_digits(
    DenialSource::Proxy { code: "٥٠٠".to_owned() },
    "Unicode digits"
)]
#[tokio::test]
async fn a_client_denial_answers_invalid_request_with_the_rejected_text(
    #[case] source: DenialSource,
    #[case] reason: &str,
) {
    let outcome = run_with(JWT, prerequisites(), Err(denial(source, reason, None))).await;
    let refusal = outcome.result.unwrap_err();
    assert_eq!(refusal.error, RefusalCode::InvalidRequest);
    assert_eq!(refusal.description, REJECTED_SUBJECT_TOKEN);
    assert!(!refusal.description.contains(reason));
    assert_eq!(
        outcome.logs,
        vec![(
            "WARNING".to_owned(),
            format!("token exchange refused a subject_token: {reason}")
        )]
    );
}

#[rstest]
#[case::http_500(DenialSource::Http { status: 500 })]
#[case::http_503(DenialSource::Http { status: 503 })]
#[case::proxy_500(DenialSource::Proxy { code: "500".to_owned() })]
#[case::proxy_padded(DenialSource::Proxy { code: "0500".to_owned() })]
#[case::proxy_599(DenialSource::Proxy { code: "599".to_owned() })]
#[case::proxy_1000(DenialSource::Proxy { code: "1000".to_owned() })]
#[case::proxy_99999(DenialSource::Proxy { code: "99999".to_owned() })]
#[tokio::test]
async fn a_server_denial_answers_temporarily_unavailable(#[case] source: DenialSource) {
    let outcome = run_with(
        JWT,
        prerequisites(),
        Err(denial(source, UNAVAILABLE_REASON, None)),
    )
    .await;
    let refusal = outcome.result.unwrap_err();
    assert_eq!(refusal.error, RefusalCode::TemporarilyUnavailable);
    assert_eq!(refusal.description, SUBJECT_CHECK_UNAVAILABLE);
    assert!(!refusal.description.contains(UNAVAILABLE_REASON));
    assert_eq!(
        outcome.logs,
        vec![(
            "ERROR".to_owned(),
            format!(
                "token exchange could not verify a subject_token, retryable: {UNAVAILABLE_REASON}"
            )
        )]
    );
}

#[tokio::test]
async fn a_transient_database_fault_outranks_a_client_error_status() {
    let cause = fault(ErrorType::Connect, "the database connection was refused");
    let outcome = run_with(
        JWT,
        prerequisites(),
        Err(denial(
            DenialSource::Http { status: 401 },
            "the IdP said no",
            Some(cause),
        )),
    )
    .await;
    let refusal = outcome.result.unwrap_err();
    assert_eq!(refusal.error, RefusalCode::TemporarilyUnavailable);
    assert_eq!(refusal.description, SUBJECT_CHECK_UNAVAILABLE);
    assert_eq!(
        outcome.logs,
        vec![(
            "ERROR".to_owned(),
            "token exchange could not verify a subject_token, retryable: the IdP said no"
                .to_owned()
        )]
    );
}

#[tokio::test]
async fn a_permanent_database_fault_answers_the_faulted_text() {
    let cause = fault(ErrorType::Prisma, "the query engine reported a fault");
    let outcome = run_with(
        JWT,
        prerequisites(),
        Err(denial(
            DenialSource::Http { status: 401 },
            "the IdP said no",
            Some(cause),
        )),
    )
    .await;
    let refusal = outcome.result.unwrap_err();
    assert_eq!(refusal.error, RefusalCode::TemporarilyUnavailable);
    assert_eq!(refusal.description, SUBJECT_CHECK_FAULTED);
    assert_eq!(
        outcome.logs,
        vec![(
            "ERROR".to_owned(),
            "token exchange could not verify a subject_token, faulted: the IdP said no".to_owned()
        )]
    );
}

#[tokio::test]
async fn a_permanent_fault_below_a_transient_one_wins() {
    let cause = with_cause(
        fault(ErrorType::Connect, "the reconnect failed"),
        fault(ErrorType::Prisma, "the query engine binary is missing"),
    );
    let outcome = run_with(
        JWT,
        prerequisites(),
        Err(denial(DenialSource::Plain, "the IdP said no", Some(cause))),
    )
    .await;
    let refusal = outcome.result.unwrap_err();
    assert_eq!(refusal.error, RefusalCode::TemporarilyUnavailable);
    assert_eq!(refusal.description, SUBJECT_CHECK_FAULTED);
    assert_eq!(
        outcome.logs,
        vec![(
            "ERROR".to_owned(),
            "token exchange could not verify a subject_token, faulted: the IdP said no".to_owned()
        )]
    );
}

#[tokio::test]
async fn the_exchange_reads_the_prerequisites_on_every_call() {
    let reads = Arc::new(AtomicUsize::new(0));
    let exchange = IdpTokenExchange {
        inputs: CountingInputs {
            prerequisites: prerequisites(),
            reads: Arc::clone(&reads),
        },
        authorizer: FakeAuthorizer {
            answer: Ok(subject(Some("user-1"), None, None)),
            seen: Arc::new(Mutex::new(Vec::new())),
        },
    };
    exchange.exchange(JWT, &()).await.unwrap();
    exchange.exchange(JWT, &()).await.unwrap();
    assert_eq!(reads.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn the_exchange_answers_through_the_b10063_port() {
    let exchange = IdpTokenExchange {
        inputs: FixedInputs {
            prerequisites: prerequisites(),
        },
        authorizer: FakeAuthorizer {
            answer: Ok(subject(Some("user-1"), Some("team-1"), None)),
            seen: Arc::new(Mutex::new(Vec::new())),
        },
    };
    let exchanged: Result<SubjectIdentity, SubjectTokenRefusal> =
        ExchangeSubjectToken::exchange(&exchange, JWT, &()).await;
    assert_eq!(
        exchanged.unwrap(),
        SubjectIdentity {
            user_id: "user-1".to_owned(),
            team_id: Some("team-1".to_owned()),
        }
    );
}
