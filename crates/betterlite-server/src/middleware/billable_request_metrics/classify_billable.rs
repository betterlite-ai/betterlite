mod tests;

use axum::http::Method;
use betterlite_billing::billable_category::BillableCategory;

use super::classify_a2a::classify_a2a_route;
use super::classify_llm::classify_llm_route;
use super::classify_mcp::classify_mcp_route;

pub(super) fn classify_billable_request(
    path: &str,
    method: &Method,
) -> Option<(BillableCategory, &'static str)> {
    let trimmed = path.trim_end_matches('/');
    let normalized = if trimmed.is_empty() { "/" } else { trimmed };

    if let Some(route) = classify_mcp_route(normalized) {
        return Some((BillableCategory::Mcp, route));
    }
    if let Some(route) = classify_a2a_route(normalized) {
        return Some((BillableCategory::A2a, route));
    }
    // POST-only is a conservative gate: a non-POST SpendLogs row never bills, so any mismatch undercounts rather than overcounts.
    if method != Method::POST {
        return None;
    }
    classify_llm_route(normalized).map(|route| (BillableCategory::Llm, route))
}
