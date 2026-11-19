use super::log::{IssuerBackfillEvent, IssuerBackfillLog};
use super::{IssuerBackfillTable, IssuerCandidateRow, is_stamped_issuer_row};

// Per-row best effort: a row whose write fails is logged and retried at the next boot.
pub async fn heal_stamped_issuers<T: IssuerBackfillTable>(
    table: &T,
    log: &dyn IssuerBackfillLog,
) -> Result<usize, T::Error> {
    let candidates = table.discovery_stamped_rows().await?;
    let stamped: Vec<&IssuerCandidateRow> = candidates
        .iter()
        .filter(|row| is_stamped_issuer_row(row))
        .collect();
    if stamped.is_empty() {
        return Ok(0);
    }
    let mut healed = 0_usize;
    for row in stamped {
        match table.clear_issuer(&row.server_id).await {
            Ok(()) => {
                healed += 1;
                log.issuer_event(IssuerBackfillEvent::Healed {
                    server_id: row.server_id.as_str(),
                    issuer: row.issuer.as_deref().unwrap_or_default(),
                    alias: row
                        .alias
                        .as_deref()
                        .filter(|value| !value.is_empty())
                        .or(row.server_name.as_deref()),
                });
            }
            Err(error) => {
                let detail = error.to_string();
                log.issuer_event(IssuerBackfillEvent::HealFailed {
                    server_id: row.server_id.as_str(),
                    error: &detail,
                });
            }
        }
    }
    if healed > 0 {
        log.issuer_event(IssuerBackfillEvent::HealedCount { healed });
    }
    Ok(healed)
}
