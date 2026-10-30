use betterlite_db::database_faults::{self, DbError};
use serde::{Deserialize, Serialize};

pub(crate) struct Op;

#[derive(Deserialize)]
pub(crate) struct Input {
    error: DbError,
}

#[derive(Serialize)]
struct Output {
    #[serde(flatten)]
    availability: Availability,
    #[serde(flatten)]
    data: DataClasses,
    #[serde(flatten)]
    transport: TransportClasses,
    sqlstate: Option<String>,
    #[serde(flatten)]
    transaction: TransactionDiagnostics,
    #[serde(flatten)]
    engine: EngineDiagnostics,
}

#[derive(Serialize)]
struct Availability {
    connection: bool,
    infrastructure: bool,
}

#[derive(Serialize)]
struct DataClasses {
    data: bool,
    prisma: bool,
}

#[derive(Serialize)]
struct TransportClasses {
    transport: bool,
    deadlock: bool,
}

#[derive(Serialize)]
struct TransactionDiagnostics {
    read_only: bool,
    permanent: bool,
}

#[derive(Serialize)]
struct EngineDiagnostics {
    engine_internal: bool,
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let error = &input.error;
        let output = Output {
            availability: Availability {
                connection: database_faults::is_database_connection_error(error),
                infrastructure: database_faults::is_database_infrastructure(error),
            },
            data: DataClasses {
                data: database_faults::is_prisma_data_error(error),
                prisma: database_faults::is_prisma_error(error),
            },
            transport: TransportClasses {
                transport: database_faults::is_database_transport_error(error),
                deadlock: database_faults::is_deadlock_error(error),
            },
            sqlstate: database_faults::postgres_sqlstate(error),
            transaction: TransactionDiagnostics {
                read_only: database_faults::is_read_only_transaction_error(error),
                permanent: database_faults::is_permanent_database_fault(error),
            },
            engine: EngineDiagnostics {
                engine_internal: database_faults::is_prisma_engine_internal(error),
            },
        };
        serde_json::to_value(output).map_err(|source| {
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
