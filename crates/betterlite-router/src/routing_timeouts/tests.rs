#![cfg(test)]

use std::time::{Duration, Instant};

use super::global::GlobalRequestTimeout;
use super::plan::{DeploymentTimeouts, RequestTimeouts, RouterTimeouts};
use super::seconds::Seconds;

mod routing_timeouts_behaviors;
mod routing_timeouts_boundaries;

fn seconds(value: f64) -> Seconds {
    Seconds::new("test", value).unwrap()
}

fn global_timeout(value: f64, explicitly_set: bool) -> GlobalRequestTimeout {
    GlobalRequestTimeout {
        seconds: seconds(value),
        explicitly_set,
    }
}

fn router_settings(timeout: f64) -> RouterTimeouts {
    RouterTimeouts {
        timeout: seconds(timeout),
        stream_timeout: None,
        request_timeout: None,
        default_stream_timeout: None,
    }
}

fn request_settings(stream: bool) -> RequestTimeouts {
    RequestTimeouts {
        stream,
        timeout: None,
        request_timeout: None,
        stream_timeout: None,
    }
}

fn no_deployment() -> DeploymentTimeouts {
    DeploymentTimeouts::default()
}

fn at(base: Instant, seconds: u64) -> Instant {
    base.checked_add(Duration::from_secs(seconds)).unwrap()
}
