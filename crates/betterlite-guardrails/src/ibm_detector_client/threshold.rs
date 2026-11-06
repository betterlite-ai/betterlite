use super::config::IbmDetectorConfig;
use super::wire::Detection;

mod tests;

#[must_use]
pub fn is_violation(config: &IbmDetectorConfig, detection: &Detection) -> bool {
    let Some(threshold) = config.score_threshold else {
        return true;
    };
    detection.score >= threshold
}

#[must_use]
pub fn filter_detections<'detections>(
    config: &IbmDetectorConfig,
    detections: &'detections [Detection],
) -> Vec<&'detections Detection> {
    detections
        .iter()
        .filter(|detection| is_violation(config, detection))
        .collect()
}
