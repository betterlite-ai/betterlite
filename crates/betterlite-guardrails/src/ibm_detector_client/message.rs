use super::config::IbmDetectorConfig;
use super::threshold::filter_detections;
use super::wire::Detection;

mod tests;

#[must_use]
pub fn detector_server_message(
    config: &IbmDetectorConfig,
    detections_list: &[Vec<Detection>],
) -> String {
    let mut total = 0_usize;
    let mut sections = String::new();
    for (index, message_detections) in detections_list.iter().enumerate() {
        let filtered = filter_detections(config, message_detections);
        if filtered.is_empty() {
            continue;
        }
        total = total.saturating_add(filtered.len());
        let lines = filtered.iter().fold(String::new(), |mut lines, detection| {
            let line = format!(
                "  - {} (score: {:.3})\n    Text: '{}'\n",
                detection.detection_type.to_uppercase(),
                detection.score,
                detection.text,
            );
            lines.push_str(&line);
            lines
        });
        let block = format!("Message {}:\n{lines}\n", index.saturating_add(1));
        sections.push_str(&block);
    }
    format!(
        "IBM Guardrail Detector failed: {total} violation(s) detected\n\nIBM Guardrail Detector failed:\n\n{sections}"
    )
    .trim()
    .to_owned()
}

#[must_use]
pub fn orchestrator_message(config: &IbmDetectorConfig, detections: &[Detection]) -> String {
    let filtered = filter_detections(config, detections);
    let mut message = format!(
        "IBM Guardrail Detector failed: {} violation(s) detected\n\n",
        filtered.len()
    );
    for detection in filtered {
        let line = format!(
            "- {} (detector: {}, score: {:.3})\n  Text: '{}'\n\n",
            detection.detection_type.to_uppercase(),
            detection
                .detector_id
                .as_deref()
                .unwrap_or(&config.detector_id),
            detection.score,
            detection.text,
        );
        message.push_str(&line);
    }
    message.trim().to_owned()
}
