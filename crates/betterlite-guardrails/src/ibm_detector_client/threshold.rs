use super::config::IbmDetectorConfig;
use super::wire::Detection;

mod tests;

#[must_use]
pub fn filter_detections<'detections>(
    config: &IbmDetectorConfig,
    detections: &'detections [Detection],
) -> Vec<&'detections Detection> {
    let Some(threshold) = config.score_threshold else {
        return detections.iter().collect();
    };
    detections
        .iter()
        .filter(|detection| detection.score.unwrap_or(0.0_f64) >= threshold)
        .collect()
}
