use betterlite_config::sections::content_filter_records::overrides::CompetitorIntentResult;

use super::{BaseCompetitorIntentChecker, Meaning, normalize};

// The workspace denies multiple_inherent_impl, so LiteLLM's base run lives on this trait.
pub trait BaseCompetitorRun {
    #[must_use]
    fn run(&self, text: &str) -> CompetitorIntentResult;
}

impl BaseCompetitorRun for BaseCompetitorIntentChecker {
    fn run(&self, text: &str) -> CompetitorIntentResult {
        self.evaluate(&normalize(text), base_classify)
    }
}

// ast-grep-ignore: no-one-line-functions - evaluate takes this classifier as the function item it calls
const fn base_classify(_text: &str, _token: &str) -> (Meaning, f64) {
    (Meaning::OtherMeaning, 0.5_f64)
}
