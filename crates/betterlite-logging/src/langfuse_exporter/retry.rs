use std::time::Duration;

// 2^6 seconds: the exponent stops growing there, so every later retry waits the same 64 s
const MAX_BACKOFF_SECONDS: u64 = 64;

// Every batch settles as delivered, rejected, or too large; None from post() means the request is worth re-sending
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeliveryOutcome {
    Delivered,
    Rejected,
    TooLarge,
}

#[must_use]
pub fn retry_delays(retries: u32) -> Vec<Duration> {
    (0..retries)
        .map(|attempt| Duration::from_secs(backoff_seconds(attempt)))
        .collect()
}

const fn backoff_seconds(attempt: u32) -> u64 {
    match attempt {
        0 => 1,
        1 => 2,
        2 => 4,
        3 => 8,
        4 => 16,
        5 => 32,
        _ => MAX_BACKOFF_SECONDS,
    }
}
