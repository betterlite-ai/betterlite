use betterlite_db::database_faults::{self, DbError, ErrorType};
use serde::{Deserialize, Serialize};

pub(crate) struct Op;

#[derive(Deserialize)]
pub(crate) struct RawDbError {
    error_type: ErrorType,
    #[serde(default)]
    message: String,
    #[serde(default)]
    code: Option<String>,
    #[serde(default)]
    meta_code: Option<serde_json::Value>,
    #[serde(default)]
    traceback_modules: Vec<String>,
}

impl From<&RawDbError> for DbError {
    fn from(raw: &RawDbError) -> Self {
        Self {
            error_type: raw.error_type,
            message: raw.message.clone(),
            code: raw.code.clone(),
            meta_code: raw
                .meta_code
                .as_ref()
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned),
            traceback_modules: raw.traceback_modules.clone(),
            cause: None,
        }
    }
}

#[derive(Deserialize)]
#[serde(untagged)]
pub(crate) enum Input {
    Classification { error: RawDbError },
    Matrix { errors: Vec<RawDbError> },
    Retry { scenarios: Vec<retry::Script> },
}

mod capture;
mod retry;

#[derive(Serialize)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "mirrors LiteLLM's predicate set"
)]
struct Output {
    connection: bool,
    infrastructure: bool,
    data: bool,
    prisma: bool,
    transport: bool,
    sqlstate: Option<String>,
    deadlock: bool,
    read_only: bool,
    permanent: bool,
    engine_internal: bool,
}

impl From<&DbError> for Output {
    fn from(error: &DbError) -> Self {
        let classes = database_faults::classify(error.error_type);
        Self {
            connection: classes.connection,
            infrastructure: classes.infrastructure,
            data: error.error_type == ErrorType::Data,
            prisma: classes.prisma,
            transport: database_faults::is_transport_error(error),
            sqlstate: database_faults::postgres_sqlstate(error),
            deadlock: database_faults::is_deadlock_error(error),
            read_only: database_faults::is_read_only_transaction_error(error),
            permanent: database_faults::is_permanent_database_fault(error),
            engine_internal: database_faults::is_prisma_engine_internal(error),
        }
    }
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let output = match input {
            Input::Classification { error } => {
                serde_json::to_value(Output::from(&DbError::from(error)))
            }
            Input::Matrix { errors } => serde_json::to_value(
                errors
                    .iter()
                    .map(|raw| Output::from(&DbError::from(raw)))
                    .collect::<Vec<_>>(),
            ),
            Input::Retry { scenarios } => {
                let reports = scenarios
                    .iter()
                    .map(retry::run)
                    .collect::<Result<Vec<_>, _>>()?;
                serde_json::to_value(reports)
            }
        };
        output.map_err(|source| {
            Box::new(crate::helpers::HelperError {
                class: "SerializationError".to_owned(),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message: source.to_string(),
            })
        })
    }
}
