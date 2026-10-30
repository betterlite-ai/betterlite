use betterlite_db::database_faults::{self, DbError};

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = serde_json::Value;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let error: DbError = serde_json::from_value(
            input.get("error").cloned().unwrap_or_default(),
        )
        .map_err(|source| {
            Box::new(crate::helpers::HelperError {
                class: "InvalidInput".to_owned(),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message: source.to_string(),
            })
        })?;
        Ok(serde_json::json!({
            "connection": database_faults::is_database_connection_error(&error),
            "infrastructure": database_faults::is_database_infrastructure(&error),
            "data": database_faults::is_prisma_data_error(&error),
            "transport": database_faults::is_database_transport_error(&error),
            "prisma": database_faults::is_prisma_error(&error),
            "deadlock": database_faults::is_deadlock_error(&error),
            "sqlstate": database_faults::postgres_sqlstate(&error),
            "read_only": database_faults::is_read_only_transaction_error(&error),
            "engine_internal": database_faults::is_prisma_engine_internal(&error),
            "permanent": database_faults::is_permanent_database_fault(&error),
        }))
    }
}
