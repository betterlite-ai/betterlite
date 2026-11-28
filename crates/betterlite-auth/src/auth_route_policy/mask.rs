use betterlite_redact::structured::SensitiveDataMasker;

const SHORT_ID_LIMIT: usize = 4;
const SHORT_ID_MASK: &str = "***";
const VISIBLE_PREFIX: usize = 6;
const VISIBLE_SUFFIX: usize = 2;
const NON_SENSITIVE: [&str; 1] = ["cost"];

// RouteChecks._mask_user_id: a short id is hidden whole, a longer one keeps six leading and two trailing characters
#[must_use]
pub fn mask_user_id(user_id: &str) -> String {
    if user_id.chars().count() <= SHORT_ID_LIMIT {
        return SHORT_ID_MASK.to_owned();
    }
    let mut masker = SensitiveDataMasker::with_patterns(Vec::<String>::new(), NON_SENSITIVE);
    masker.visible_prefix = VISIBLE_PREFIX;
    masker.visible_suffix = VISIBLE_SUFFIX;
    masker.mask_value(user_id)
}
