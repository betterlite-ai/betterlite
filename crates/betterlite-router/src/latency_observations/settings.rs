#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Settings {
    pub ttl_seconds: f64,
    pub lowest_latency_buffer: f64,
    pub max_latency_list_size: i64,
    pub ttft_percentile: Option<f64>,
}

// RoutingArgs: the four routing_strategy_args the lowest-latency handler validates when it is built
impl Default for Settings {
    fn default() -> Self {
        Self {
            ttl_seconds: 3_600.0_f64,
            lowest_latency_buffer: 0.0_f64,
            max_latency_list_size: 10,
            ttft_percentile: None,
        }
    }
}
