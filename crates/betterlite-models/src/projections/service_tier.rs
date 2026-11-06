pub const KNOWN_REQUEST_SERVICE_TIERS: &[&str] = &[
    "auto",
    "balanced",
    "batch",
    "default",
    "fast",
    "flex",
    "priority",
    "scale",
    "standard",
    "standard_only",
    "ultrafast",
];

#[must_use]
pub fn resolved_service_tier<'request>(
    response_tier: Option<&'request str>,
    usage_tier: Option<&'request str>,
    requested_tier: Option<&'request str>,
) -> Option<&'request str> {
    if let Some(tier) = response_tier.or(usage_tier) {
        return Some(tier);
    }
    requested_tier.filter(|tier| KNOWN_REQUEST_SERVICE_TIERS.contains(tier))
}

mod tests;
