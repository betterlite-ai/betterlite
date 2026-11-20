use betterlite_config::sections::content_filter_records::overrides::CompetitorIntentResult;

use super::{BaseCompetitorIntentChecker, Meaning, normalize};

impl BaseCompetitorIntentChecker {
    // LiteLLM's BaseCompetitorIntentChecker.run; the base classifier treats every token as other meaning
    #[must_use]
    pub fn run(&self, text: &str) -> CompetitorIntentResult {
        self.evaluate(&normalize(text), base_classify)
    }
}

fn base_classify(_text: &str, _token: &str) -> (Meaning, f64) {
    (Meaning::OtherMeaning, base_confidence())
}

// the other-meaning branch discards this confidence, so no caller can observe a different value
#[cfg_attr(test, mutants::skip)]
const fn base_confidence() -> f64 {
    0.5_f64
}
