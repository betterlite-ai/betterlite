use std::cmp::Reverse;
use std::collections::BTreeMap;

use betterlite_db::gateway_requests::GatewayRequestActivityResponse;
use betterlite_db::gateway_requests::GatewayRequestBreakdownEntry;
use betterlite_db::gateway_requests::GatewayRequestDailyEntry;
use betterlite_db::gateway_requests::gateway_request_counts::GatewayRequestCounts;

use super::store::GatewayActivityRow;

pub(super) fn activity_response(rows: &[GatewayActivityRow]) -> GatewayRequestActivityResponse {
    let mut totals = GatewayRequestCounts::default();
    let mut dates: BTreeMap<&str, GatewayRequestCounts> = BTreeMap::new();
    let mut routes: BTreeMap<(&str, &str), GatewayRequestCounts> = BTreeMap::new();
    for row in rows {
        let counts = GatewayRequestCounts {
            successful_requests: row.successful_requests,
            failed_requests: row.failed_requests,
        };
        totals = totals.fold(counts);
        let date = dates.entry(row.date.as_str()).or_default();
        *date = date.fold(counts);
        let route = routes
            .entry((row.category.as_str(), row.route.as_str()))
            .or_default();
        *route = route.fold(counts);
    }
    let mut by_route: Vec<GatewayRequestBreakdownEntry> = routes
        .into_iter()
        .map(|((category, route), counts)| GatewayRequestBreakdownEntry {
            category: category.to_owned(),
            route: route.to_owned(),
            successful_requests: counts.successful_requests,
            failed_requests: counts.failed_requests,
        })
        .collect();
    by_route.sort_by_key(|entry| Reverse(entry.successful_requests));
    GatewayRequestActivityResponse {
        total_successful_requests: totals.successful_requests,
        total_failed_requests: totals.failed_requests,
        by_date: dates
            .into_iter()
            .map(|(date, counts)| GatewayRequestDailyEntry {
                date: date.to_owned(),
                successful_requests: counts.successful_requests,
                failed_requests: counts.failed_requests,
            })
            .collect(),
        by_route,
    }
}
