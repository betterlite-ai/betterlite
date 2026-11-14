use betterlite_db::database_repair::RepairError;
use betterlite_db::database_repair::migration::{
    ImportEvent, ImportReport, import_verification_tokens,
};
use betterlite_db::database_repair::unassigned::{PRODUCTION_WARNING, apply_db_fixes};
use betterlite_db::migration_ledger::connection::connect;
use betterlite_db_admin::command_shell::{DbAdminFuture, DbAdminService, ResolvedConfig};
use betterlite_db_admin::commands::{DbAdminCommand, DbAdminError, DbAdminOutcome};

pub(crate) struct DatabaseRepairService;

impl DbAdminService for DatabaseRepairService {
    fn run(&self, command: DbAdminCommand, config: ResolvedConfig) -> DbAdminFuture<'_> {
        Box::pin(async move {
            let session = connect(&config.database_url)
                .await
                .map_err(|source| RepairError::Connection(Box::new(source)))?;
            match command {
                DbAdminCommand::UpdateUnassignedTeams => {
                    let updated = apply_db_fixes(&session).await.map_err(RepairError::Teams)?;
                    Ok(DbAdminOutcome::Views {
                        lines: vec![
                            PRODUCTION_WARNING.to_owned(),
                            format!("Updated unassigned teams, Response={updated}"),
                        ],
                    })
                }
                DbAdminCommand::MigrateVerificationTokens(csv) => {
                    let report = import_verification_tokens(&session, &csv)
                        .await
                        .map_err(RepairError::Import)?;
                    Ok(DbAdminOutcome::Views {
                        lines: report_lines(&report),
                    })
                }
                other @ (DbAdminCommand::Migrate(_)
                | DbAdminCommand::DatabaseViews
                | DbAdminCommand::DailyToolSpend(_)
                | DbAdminCommand::KeyTotalSpend
                | DbAdminCommand::KeyTotalSpendFromLogs
                | DbAdminCommand::SpendPartition(_)) => Err(DbAdminError::Unregistered {
                    operation: other.operation().name(),
                }),
            }
        })
    }
}

fn report_lines(report: &ImportReport) -> Vec<String> {
    let mut lines: Vec<String> = report.events.iter().map(event_line).collect();
    lines.push("Migration completed!".to_owned());
    lines.push(format!(
        "Successfully processed: {} records",
        report.migrated()
    ));
    lines.push(format!("Errors encountered: {} records", report.failed()));
    lines
}

fn event_line(event: &ImportEvent) -> String {
    match event {
        ImportEvent::Migrated { token } => format!("Successfully migrated token: {token}"),
        ImportEvent::Skipped { token } => format!("Token {token} already exists, skipping..."),
        ImportEvent::Failed { token, source } => {
            let name = token.as_deref().unwrap_or("unknown");
            format!("Error processing row with token {name}: {source}")
        }
    }
}

mod tests;
