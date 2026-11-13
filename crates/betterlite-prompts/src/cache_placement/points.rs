use crate::cache_accounting::points::{CacheControlInjectionPoint, MessageInjectionPoint};

#[must_use]
pub const fn message_point(point: &CacheControlInjectionPoint) -> Option<&MessageInjectionPoint> {
    match point {
        CacheControlInjectionPoint::Message(message) => Some(message),
        CacheControlInjectionPoint::ToolConfig(_) => None,
    }
}

#[must_use]
pub const fn first_dialect(points: &[CacheControlInjectionPoint]) -> Option<bool> {
    match points.first() {
        Some(CacheControlInjectionPoint::Message(point)) => point.openai_dialect,
        Some(CacheControlInjectionPoint::ToolConfig(point)) => point.openai_dialect,
        None => None,
    }
}

#[must_use]
pub const fn first_external(points: &[CacheControlInjectionPoint]) -> Option<usize> {
    match points.first() {
        Some(CacheControlInjectionPoint::Message(point)) => point.external_breakpoints,
        Some(CacheControlInjectionPoint::ToolConfig(point)) => point.external_breakpoints,
        None => None,
    }
}
