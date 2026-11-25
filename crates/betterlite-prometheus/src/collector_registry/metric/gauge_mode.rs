// Gauge._MULTIPROC_MODES: LiteLLM builds its gauges with all (the default), livesum and livemax
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum GaugeMode {
    All,
    LiveSum,
    LiveMax,
}

impl GaugeMode {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::LiveSum => "livesum",
            Self::LiveMax => "livemax",
        }
    }
}
