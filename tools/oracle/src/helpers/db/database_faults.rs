use betterlite_db::database_faults::{self, DbError};
use serde::{Deserialize, Serialize};

pub(crate) struct Op;

#[derive(Deserialize)]
pub(crate) struct Input {
    error: DbError,
}

#[derive(Serialize)]
struct Output {
    connection: bool,
    infrastructure: bool,
    data: bool,
    transport: bool,
    prisma: bool,
    deadlock: bool,
    sqlstate: Option<String>,
    read_only: bool,
    engine_internal: bool,
    permanent: bool,
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let error = &input.error;
        let output = Output {
            connection: database_faults::is_database_connection_error(error),
            infrastructure: database_faults::is_database_infrastructure(error),
            data: database_faults::is_prisma_data_error(error),
            transport: database_faults::is_database_transport_error(error),
            prisma: database_faults::is_prisma_error(error),
            deadlock: database_faults::is_deadlock_error(error),
            sqlstate: database_faults::postgres_sqlstate(error),
            read_only: database_faults::is_read_only_transaction_error(error),
            engine_internal: database_faults::is_prisma_engine_internal(error),
            permanent: database_faults::is_permanent_database_fault(error),
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
