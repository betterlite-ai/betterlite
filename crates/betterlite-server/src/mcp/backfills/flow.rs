use std::collections::{BTreeMap, BTreeSet};

use betterlite_db::mcp_user_credentials::decode::{decode_user_credential, parse_oauth_payload};
use betterlite_db::stored_cipher::DbCipher;
use betterlite_mcp::server_record::OAuth2Flow;

use super::log::{BackfillEvent, BackfillLog};
use super::{
    BackfillRule, FlowBackfillTable, FlowCandidateRow, FlowClassification, FlowSignals,
    classify_null_flow_row, decrypted_credentials,
};

type ClassifiedRow<'row> = (&'row FlowCandidateRow, FlowClassification);

pub async fn backfill_null_oauth2_flows<T>(
    table: &T,
    cipher: &DbCipher,
    log: &dyn BackfillLog,
) -> Result<BTreeMap<BackfillRule, usize>, T::Error>
where
    T: FlowBackfillTable,
{
    let null_rows = table.null_flow_rows().await?;
    if null_rows.is_empty() {
        return Ok(BTreeMap::new());
    }
    let server_ids: Vec<String> = null_rows.iter().map(|row| row.server_id.clone()).collect();
    let interactive = interactive_server_ids(table, &server_ids, cipher).await?;
    let (classified, counts) = classify_rows(&null_rows, &interactive, cipher, log);
    stamp_rows(table, &classified).await?;
    log.event(BackfillEvent::FlowProcessed {
        rows: null_rows.len(),
        counts: &counts,
    });
    Ok(counts)
}

// Only an interactive sign-in mints a per-user token, so a readable oauth2 credential proves the interactive flow.
async fn interactive_server_ids<T>(
    table: &T,
    server_ids: &[String],
    cipher: &DbCipher,
) -> Result<BTreeSet<String>, T::Error>
where
    T: FlowBackfillTable,
{
    let token_rows = table.user_credentials(server_ids).await?;
    Ok(token_rows
        .iter()
        .filter(|row| {
            let decoded = decode_user_credential(cipher, &row.credential_b64);
            parse_oauth_payload(decoded.as_ref()).is_ok_and(|payload| payload.is_some())
        })
        .map(|row| row.server_id.clone())
        .collect())
}

fn classify_rows<'row>(
    null_rows: &'row [FlowCandidateRow],
    interactive: &BTreeSet<String>,
    cipher: &DbCipher,
    log: &dyn BackfillLog,
) -> (Vec<ClassifiedRow<'row>>, BTreeMap<BackfillRule, usize>) {
    let mut classified: Vec<ClassifiedRow<'row>> = Vec::with_capacity(null_rows.len());
    let mut counts: BTreeMap<BackfillRule, usize> = BTreeMap::new();
    for row in null_rows {
        let credentials = decrypted_credentials(row.credentials.as_ref(), cipher);
        let classification = classify_null_flow_row(&FlowSignals {
            has_per_user_tokens: interactive.contains(row.server_id.as_str()),
            authorization_url: row.authorization_url.as_deref(),
            registration_url: row.registration_url.as_deref(),
            token_url: row.token_url.as_deref(),
            credentials: credentials.as_ref(),
        });
        let count = counts.entry(classification.rule).or_insert(0);
        *count = count.saturating_add(1);
        log.event(match classification.flow {
            Some(flow) => BackfillEvent::FlowStamped {
                server_id: row.server_id.as_str(),
                flow,
                rule: classification.rule,
            },
            None => BackfillEvent::FlowAmbiguous {
                server_id: row.server_id.as_str(),
            },
        });
        classified.push((row, classification));
    }
    (classified, counts)
}

// All rows of one flow are stamped in one batch, as LiteLLM's update_many does.
async fn stamp_rows<T>(table: &T, classified: &[ClassifiedRow<'_>]) -> Result<(), T::Error>
where
    T: FlowBackfillTable,
{
    let mut stamped_flows: Vec<OAuth2Flow> = Vec::new();
    for (_row, classification) in classified {
        if let Some(flow) = classification.flow
            && !stamped_flows.contains(&flow)
        {
            stamped_flows.push(flow);
        }
    }
    for flow in stamped_flows {
        let server_ids_for_flow: Vec<String> = classified
            .iter()
            .filter(|(_row, classification)| classification.flow == Some(flow))
            .map(|(row, _classification)| row.server_id.clone())
            .collect();
        table.stamp_flow(&server_ids_for_flow, flow).await?;
    }
    Ok(())
}
