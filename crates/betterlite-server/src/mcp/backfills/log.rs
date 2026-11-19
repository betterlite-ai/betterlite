use std::collections::BTreeMap;

use betterlite_mcp::server_record::OAuth2Flow;

use super::{BackfillRule, flow_name};

pub enum BackfillEvent<'event> {
    FlowAmbiguous {
        server_id: &'event str,
    },
    FlowStamped {
        server_id: &'event str,
        flow: OAuth2Flow,
        rule: BackfillRule,
    },
    FlowProcessed {
        rows: usize,
        counts: &'event BTreeMap<BackfillRule, usize>,
    },
    IssuerHealFailed {
        server_id: &'event str,
        error: &'event str,
    },
    IssuerHealed {
        server_id: &'event str,
        issuer: &'event str,
        alias: Option<&'event str>,
    },
    IssuerHealedCount {
        healed: usize,
    },
}

pub trait BackfillLog: Send + Sync {
    fn event(&self, event: BackfillEvent<'_>);
}

#[derive(Debug, Default)]
pub struct TracingBackfillLog;

impl BackfillLog for TracingBackfillLog {
    fn event(&self, event: BackfillEvent<'_>) {
        match event {
            BackfillEvent::FlowAmbiguous { server_id } => tracing::warn!(
                server_id,
                "oauth2_flow backfill: server is ambiguous (client credentials + token_url, no interactive signal); left unstamped"
            ),
            BackfillEvent::FlowStamped {
                server_id,
                flow,
                rule,
            } => tracing::info!(
                server_id,
                flow = flow_name(flow),
                rule = rule.as_str(),
                "oauth2_flow backfill: stamped row"
            ),
            BackfillEvent::FlowProcessed { rows, counts } => tracing::info!(
                rows,
                rules = counts.len(),
                "oauth2_flow backfill: processed oauth2 server rows"
            ),
            BackfillEvent::IssuerHealFailed { server_id, error } => tracing::warn!(
                server_id,
                error,
                "MCP issuer stamp backfill: could not heal server"
            ),
            BackfillEvent::IssuerHealed {
                server_id,
                issuer,
                alias,
            } => tracing::warn!(
                server_id,
                issuer,
                alias,
                "MCP issuer stamp backfill: cleared issuer; the configured endpoints apply again"
            ),
            BackfillEvent::IssuerHealedCount { healed } => tracing::warn!(
                healed,
                "MCP issuer stamp backfill: healed servers whose issuer discovery had written"
            ),
        }
    }
}
