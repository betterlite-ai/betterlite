use betterlite_logging::cost_breakdown::{CostBreakdownInput, CostBreakdownTarget};

// Ports _store_cost_breakdown_in_logging_obj: a missing logging target is a no-op and a storage failure never changes the charge.
pub fn store_cost_breakdown<Target>(target: Option<&mut Target>, input: &CostBreakdownInput)
where
    Target: CostBreakdownTarget,
{
    let Some(destination) = target else {
        return;
    };
    if let Err(failure) = destination.set_cost_breakdown(input) {
        tracing::debug!(?failure, "Error storing cost breakdown");
    }
}
