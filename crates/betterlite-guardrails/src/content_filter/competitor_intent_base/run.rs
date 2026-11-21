use betterlite_config::sections::content_filter_records::overrides::CompetitorIntentResult;

use super::{BaseCompetitorIntentChecker, Meaning, normalize};

impl BaseCompetitorIntentChecker {
    #[must_use]
    // ast-grep-ignore: no-one-line-functions - LiteLLM's base run is this one delegation to evaluate
    pub fn run(&self, text: &str) -> CompetitorIntentResult {
        self.evaluate(&normalize(text), base_classify)
    }
}

// ast-grep-ignore: no-one-line-functions - evaluate takes this classifier as the function item it calls
fn base_classify(_text: &str, _token: &str) -> (Meaning, f64) {
    (Meaning::OtherMeaning, 0.5_f64)
}
