use std::collections::BTreeMap;

use betterlite_mcp::server_record::OAuth2Flow;

use super::{BackfillRule, flow_name};

pub enum FlowBackfillEvent<'event> {
    Ambiguous {
        server_id: &'event str,
    },
    Stamped {
        server_id: &'event str,
        flow: OAuth2Flow,
        rule: BackfillRule,
    },
    Processed {
        rows: usize,
        counts: &'event BTreeMap<BackfillRule, usize>,
    },
}

pub trait FlowBackfillLog: Send + Sync {
    fn flow_event(&self, event: FlowBackfillEvent<'_>);
}

#[derive(Debug, Default)]
pub struct TracingFlowBackfillLog;

impl FlowBackfillLog for TracingFlowBackfillLog {
    fn flow_event(&self, event: FlowBackfillEvent<'_>) {
        match event {
            FlowBackfillEvent::Ambiguous { server_id } => tracing::warn!(
                server_id,
                "oauth2_flow backfill: server is ambiguous (client credentials + token_url, no interactive signal); left unstamped"
            ),
            FlowBackfillEvent::Stamped {
                server_id,
                flow,
                rule,
            } => tracing::info!(
                server_id,
                flow = flow_name(flow),
                rule = rule.as_str(),
                "oauth2_flow backfill: stamped row"
            ),
            FlowBackfillEvent::Processed { rows, counts } => tracing::info!(
                rows,
                rules = counts.len(),
                "oauth2_flow backfill: processed oauth2 server rows"
            ),
        }
    }
}

pub enum IssuerBackfillEvent<'event> {
    HealFailed {
        server_id: &'event str,
        error: &'event str,
    },
    Healed {
        server_id: &'event str,
        issuer: &'event str,
        alias: Option<&'event str>,
    },
    HealedCount {
        healed: usize,
    },
}

pub trait IssuerBackfillLog: Send + Sync {
    fn issuer_event(&self, event: IssuerBackfillEvent<'_>);
}

#[derive(Debug, Default)]
pub struct TracingIssuerBackfillLog;

impl IssuerBackfillLog for TracingIssuerBackfillLog {
    fn issuer_event(&self, event: IssuerBackfillEvent<'_>) {
        match event {
            IssuerBackfillEvent::HealFailed { server_id, error } => tracing::warn!(
                server_id,
                error,
                "MCP issuer stamp backfill: could not heal server"
            ),
            IssuerBackfillEvent::Healed {
                server_id,
                issuer,
                alias,
            } => tracing::warn!(
                server_id,
                issuer,
                alias,
                "MCP issuer stamp backfill: cleared issuer; the configured endpoints apply again"
            ),
            IssuerBackfillEvent::HealedCount { healed } => tracing::warn!(
                healed,
                "MCP issuer stamp backfill: healed servers whose issuer discovery had written"
            ),
        }
    }
}
