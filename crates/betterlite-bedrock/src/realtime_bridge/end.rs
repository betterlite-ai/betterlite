// The client leaving and a cancelled pump both end forwarding; only the second one is not the client's doing
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ForwardEnd {
    Ended,
    Cancelled,
}
