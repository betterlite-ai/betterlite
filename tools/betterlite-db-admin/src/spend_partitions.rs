use betterlite_db::migration_ledger::connection::connect;
use betterlite_db::spend_partitions::{SpendPartitionOperation, run_conversion};
use betterlite_db_admin::command_shell::{DbAdminFuture, DbAdminService, ResolvedConfig};
use betterlite_db_admin::commands::{
    DbAdminCommand, DbAdminError, DbAdminOutcome, SpendPartitionAdminError,
};

pub(crate) struct SpendPartitionsService;

impl DbAdminService for SpendPartitionsService {
    fn run(&self, command: DbAdminCommand, config: ResolvedConfig) -> DbAdminFuture<'_> {
        Box::pin(async move {
            let operation = partition_operation(command)?;
            let session = connect(&config.database_url).await.map_err(|source| {
                DbAdminError::SpendPartition {
                    source: SpendPartitionAdminError::Connection { source },
                }
            })?;
            run_conversion(&*session, operation).await?;
            Ok(DbAdminOutcome::Views {
                lines: vec![outcome_line(operation).to_owned()],
            })
        })
    }
}

const fn partition_operation(
    command: DbAdminCommand,
) -> Result<SpendPartitionOperation, DbAdminError> {
    match command {
        DbAdminCommand::DailyToolSpend(options) => match options.spend_partition {
            Some(operation) => Ok(operation),
            None => Err(DbAdminError::Unregistered {
                operation: options.operation().name(),
            }),
        },
        other => Err(DbAdminError::Unregistered {
            operation: other.operation().name(),
        }),
    }
}

const fn outcome_line(operation: SpendPartitionOperation) -> &'static str {
    match operation {
        SpendPartitionOperation::Partition => {
            "partition-spend-logs completed; the new LiteLLM_SpendLogs is empty and rows remain in LiteLLM_SpendLogs_legacy."
        }
        SpendPartitionOperation::Unpartition => {
            "unpartition-spend-logs completed; every surviving row now lives in the plain LiteLLM_SpendLogs."
        }
    }
}

mod tests;
