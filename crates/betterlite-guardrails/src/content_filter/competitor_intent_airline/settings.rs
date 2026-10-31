use super::super::competitor_intent_base::configuration::Settings;

#[derive(Debug, Clone, Default)]
pub struct AirlineSettings {
    pub base: Settings,
    pub other_meaning_signals: Vec<String>,
    pub competitor_signals: Vec<String>,
    pub other_meaning_anchors: Vec<String>,
    pub explicit_competitor_marker: Option<String>,
    pub explicit_other_meaning_marker: Option<String>,
}
