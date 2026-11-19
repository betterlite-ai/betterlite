mod tests;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct GatewayRequestCounts {
    pub successful_requests: i64,
    pub failed_requests: i64,
}

impl GatewayRequestCounts {
    #[must_use]
    pub const fn plus(self, succeeded: bool) -> Self {
        if succeeded {
            Self {
                successful_requests: self.successful_requests.saturating_add(1),
                failed_requests: self.failed_requests,
            }
        } else {
            Self {
                successful_requests: self.successful_requests,
                failed_requests: self.failed_requests.saturating_add(1),
            }
        }
    }

    #[must_use]
    pub const fn fold(self, other: Self) -> Self {
        Self {
            successful_requests: self
                .successful_requests
                .saturating_add(other.successful_requests),
            failed_requests: self.failed_requests.saturating_add(other.failed_requests),
        }
    }
}
