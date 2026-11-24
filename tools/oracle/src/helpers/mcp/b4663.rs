use std::fmt::Debug;
use std::future::{Future, ready};
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_db::database_faults::{DbError, ErrorType};
use betterlite_mcp::idp_token_exchange::{
    AuthorizeSubjectToken, AuthorizedSubject, DenialSource, IdpTokenExchange, PrerequisiteSwitch,
    SubjectTokenDenial, TokenExchangeInputs, TokenExchangePrerequisites,
    identity_from_subject_token,
};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::runtime::Builder;
use tracing::field::{Field, Visit};
use tracing::subscriber::set_default;
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context as LayerContext, Layer, SubscriberExt as _};
use tracing_subscriber::registry;

const PREFIX: &str = "token exchange ";

// A bool on the wire; the newtype keeps struct_excessive_bools off the case input.
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(transparent)]
struct CaseFlag(bool);

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Prerequisites {
    jwt_auth_enabled: CaseFlag,
    has_database: CaseFlag,
    maps_jwts_to_virtual_keys: CaseFlag,
}

// The case names what the proxy's JWT auth answers: a resolved result or the exception it raises.
#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum Authorizer {
    Result {
        #[serde(default)]
        user_id: Option<String>,
        #[serde(default)]
        team_id: Option<String>,
        #[serde(default)]
        agent_id: Option<String>,
    },
    Http {
        status: u16,
        detail: String,
    },
    Proxy {
        code: String,
        message: String,
    },
    Plain {
        message: String,
    },
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum CauseKind {
    Transient,
    Faulted,
}

// A database error the denial was raised from: httpx.ConnectError and prisma.errors.PrismaError on the pinned side.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Cause {
    kind: CauseKind,
    message: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    prerequisites: Prerequisites,
    subject_token: String,
    authorizer: Authorizer,
    cause: Option<Cause>,
}

struct CaseInputs<'case> {
    prerequisites: &'case Prerequisites,
}

impl TokenExchangeInputs for CaseInputs<'_> {
    fn prerequisites(&self) -> TokenExchangePrerequisites {
        TokenExchangePrerequisites {
            jwt_auth_enabled: PrerequisiteSwitch(self.prerequisites.jwt_auth_enabled.0),
            has_database: PrerequisiteSwitch(self.prerequisites.has_database.0),
            maps_jwts_to_virtual_keys: PrerequisiteSwitch(
                self.prerequisites.maps_jwts_to_virtual_keys.0,
            ),
        }
    }
}

struct CaseAuthorizer {
    answer: Result<AuthorizedSubject, Box<SubjectTokenDenial>>,
}

impl AuthorizeSubjectToken for CaseAuthorizer {
    type Context = ();

    fn authorize(
        &self,
        _subject_token: &str,
        _context: &(),
    ) -> impl Future<Output = Result<AuthorizedSubject, SubjectTokenDenial>> + Send {
        ready(self.answer.clone().map_err(|denial| *denial))
    }
}

#[derive(Clone, Default)]
struct Logs(Arc<Mutex<Vec<(String, String)>>>);

impl Logs {
    fn entries(&self) -> Vec<Value> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
            .into_iter()
            .map(|(level, message)| json!({"level": level, "message": message}))
            .collect()
    }
}

struct MessageField(Option<String>);

impl Visit for MessageField {
    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        if field.name() == "message" {
            self.0 = Some(format!("{value:?}"));
        }
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.0 = Some(value.to_owned());
        }
    }
}

impl<Current> Layer<Current> for Logs
where
    Current: Subscriber,
{
    fn on_event(&self, event: &Event<'_>, _context: LayerContext<'_, Current>) {
        let level = *event.metadata().level();
        let recorded = if level == Level::WARN {
            "WARNING"
        } else if level == Level::ERROR {
            "ERROR"
        } else {
            return;
        };
        let mut field = MessageField(None);
        event.record(&mut field);
        let Some(message) = field.0 else {
            return;
        };
        if !message.starts_with(PREFIX) {
            return;
        }
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((recorded.to_owned(), message));
    }
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let logs = Logs::default();
        let _guard = set_default(registry().with(logs.clone()));
        let runtime = Builder::new_current_thread().enable_time().build()?;
        runtime.block_on(answer(input, &logs))
    }
}

async fn answer(input: &Input, logs: &Logs) -> HelperResult {
    let exchange = IdpTokenExchange {
        inputs: CaseInputs {
            prerequisites: &input.prerequisites,
        },
        authorizer: CaseAuthorizer {
            answer: authorizer_answer(input),
        },
    };
    let outcome = identity_from_subject_token(&input.subject_token, &(), &exchange).await;
    let body = match outcome {
        Ok(identity) => json!({"user_id": identity.user_id, "team_id": identity.team_id}),
        Err(refusal) => {
            json!({"error": refusal.error.as_str(), "description": refusal.description})
        }
    };
    Ok(json!({"answer": body, "logs": logs.entries()}))
}

// boxed to keep the error under result_large_err's 128-byte limit
fn authorizer_answer(input: &Input) -> Result<AuthorizedSubject, Box<SubjectTokenDenial>> {
    match &input.authorizer {
        Authorizer::Result {
            user_id,
            team_id,
            agent_id,
        } => Ok(AuthorizedSubject {
            user_id: user_id.clone(),
            team_id: team_id.clone(),
            agent_id: agent_id.clone(),
        }),
        Authorizer::Http { status, detail } => Err(denial(
            DenialSource::Http { status: *status },
            detail,
            input.cause.as_ref(),
        )),
        Authorizer::Proxy { code, message } => Err(denial(
            DenialSource::Proxy { code: code.clone() },
            message,
            input.cause.as_ref(),
        )),
        Authorizer::Plain { message } => {
            Err(denial(DenialSource::Plain, message, input.cause.as_ref()))
        }
    }
}

fn denial(source: DenialSource, reason: &str, cause: Option<&Cause>) -> Box<SubjectTokenDenial> {
    Box::new(SubjectTokenDenial {
        source,
        reason: reason.to_owned(),
        database_cause: cause.map(database_cause),
    })
}

fn database_cause(cause: &Cause) -> DbError {
    let error_type = match cause.kind {
        CauseKind::Transient => ErrorType::Connect,
        CauseKind::Faulted => ErrorType::Prisma,
    };
    DbError {
        error_type,
        message: cause.message.clone(),
        code: None,
        meta_code: None,
        traceback_modules: Vec::new(),
        cause: None,
    }
}
