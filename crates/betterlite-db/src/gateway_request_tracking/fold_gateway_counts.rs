use crate::gateway_requests::GatewayRequestKey;
use crate::gateway_requests::GatewayRequestSnapshot;
use crate::gateway_requests::gateway_request_counts::GatewayRequestCounts;

mod tests;

#[must_use]
pub fn fold_counts<Entries>(entries: Entries) -> GatewayRequestSnapshot
where
    Entries: IntoIterator<Item = (GatewayRequestKey, GatewayRequestCounts)>,
{
    let mut folded = GatewayRequestSnapshot::new();
    for (key, counts) in entries {
        let current = folded.get(&key).copied().unwrap_or_default();
        let _replaced = folded.insert(key, current.fold(counts));
    }
    folded
}
