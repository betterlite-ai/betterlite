use super::config::Config;
use super::error::ForecastError;
use super::reply::parse_cap_reply;
use super::verdict::Verdict;
use crate::capability_math;

#[derive(Debug, Clone, PartialEq)]
pub struct Forecast {
    pub verdict: Verdict,
    pub threshold: f64,
    pub p_solve: f64,
    pub calibration_version: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Route {
    pub tier: String,
    pub meets_threshold: bool,
    pub forecast: Forecast,
}

pub fn route(config: &Config, reply: &[u8]) -> Result<Route, ForecastError> {
    let verdict = parse_cap_reply(reply)?;
    let threshold = capability_math::threshold(
        config.base_threshold,
        config.threshold_step,
        verdict.capability_boundary.extra_steps(),
    );
    let calibration = config.calibration.as_ref();
    let p_solve = calibration.map_or(verdict.p_solve, |configured| {
        capability_math::calibrate(configured.slope, configured.intercept, verdict.p_solve)
    });
    let calibration_version = calibration.map(|configured| configured.version.clone());
    let forecast = Forecast {
        verdict,
        threshold,
        p_solve,
        calibration_version,
    };
    let (tier, meets_threshold) = capability_math::select(
        forecast.p_solve,
        forecast.threshold,
        &config.efficient_tier,
        &config.capable_tier,
    );
    Ok(Route {
        tier: tier.to_owned(),
        meets_threshold,
        forecast,
    })
}
