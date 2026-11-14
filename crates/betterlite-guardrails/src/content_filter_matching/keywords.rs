use betterlite_config::sections::content_category::loading::CategoryLoadResult;
use betterlite_config::sections::content_filter_records::ContentFilterAction;
use regex::Regex;

use super::CategoryMatch;
use super::ContentFilterMatchingError;
use super::category_exception_present;
use super::global_exception_present;
use crate::pattern_matching::masking::keyword_pattern;

pub fn check_category_keywords(
    text: &str,
    exceptions: &[String],
    categories: &CategoryLoadResult,
) -> Result<Option<CategoryMatch>, ContentFilterMatchingError> {
    let text_lower = text.to_lowercase();
    if global_exception_present(exceptions, &text_lower) {
        return Ok(None);
    }
    for (keyword, entry) in &categories.always_block_category_keywords {
        if keyword_regex(keyword)?.is_match(&text_lower) {
            return Ok(Some(keyword_match(keyword, entry)));
        }
    }
    for (keyword, entry) in &categories.category_keywords {
        if !keyword_regex(keyword)?.is_match(&text_lower) {
            continue;
        }
        if category_exception_present(categories, &entry.0, &text_lower) {
            continue;
        }
        return Ok(Some(keyword_match(keyword, entry)));
    }
    Ok(None)
}

fn keyword_match(
    keyword: &str,
    (category, severity, action): &(String, String, ContentFilterAction),
) -> CategoryMatch {
    CategoryMatch {
        matched: keyword.to_owned(),
        category: category.clone(),
        severity: severity.clone(),
        action: *action,
    }
}

fn keyword_regex(keyword: &str) -> Result<Regex, regex::Error> {
    let pattern = keyword_pattern(keyword);
    if keyword.contains(' ') {
        return Regex::new(&pattern);
    }
    Regex::new(&format!(r"\b{pattern}\b"))
}
