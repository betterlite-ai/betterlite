use std::collections::BTreeMap;

use betterlite_config::sections::content_category::loading::CategoryLoadResult;
use betterlite_config::sections::content_filter_records::ContentFilterDetection;

use crate::content_filter_actions::error::ContentFilterActionError;
use crate::content_filter_actions::handlers::category_keyword::handle_category_keyword_match;
use crate::content_filter_actions::handlers::conditional::handle_conditional_match;
use crate::content_filter_actions::{ContentFilterSource, filter_single_text};
use crate::content_filter_matching::CategoryMatch;
use crate::content_filter_matching::conditional::check_conditional_categories;
use crate::content_filter_matching::keywords::check_category_keywords;
use crate::content_filter_matching::phrase::check_phrase_patterns;
use crate::content_filter_streaming::state::{CarriedMatches, DetectionCheck, StreamedDetection};

// What one scan gets: the text the open sentence makes, the buffer alone, and the prefix's first matches
pub(super) struct ScanInput<'scan> {
    pub(super) checks: bool,
    pub(super) conditional_text: &'scan str,
    pub(super) text: &'scan str,
    pub(super) carried: &'scan CarriedMatches,
}

// Run the category checks over the open sentence and the buffer, then patterns and words over the buffer
pub(super) fn filter_streamed_text(
    source: &ContentFilterSource<'_>,
    input: &ScanInput<'_>,
    detections: &mut Vec<StreamedDetection>,
) -> Result<(String, CarriedMatches), ContentFilterActionError> {
    let mut filtered = input.text.to_owned();
    let winners = if input.checks {
        CheckScan {
            categories: source.categories,
            keyword_redaction_tag: source.keyword_redaction_tag,
            text: input.text,
            filtered: &mut filtered,
            detections: &mut *detections,
        }
        .run(input)?
    } else {
        CarriedMatches::default()
    };
    mask_patterns_and_words(source, &filtered, detections).map(|masked| (masked, winners))
}

// Where a category check reads its categories and writes its mask and detections
struct CheckScan<'scan> {
    categories: &'scan CategoryLoadResult,
    keyword_redaction_tag: &'scan str,
    text: &'scan str,
    filtered: &'scan mut String,
    detections: &'scan mut Vec<StreamedDetection>,
}

impl CheckScan<'_> {
    // Each check's first match, with a dropped phrase suppressing the first two
    fn run(&mut self, input: &ScanInput<'_>) -> Result<CarriedMatches, ContentFilterActionError> {
        let conditional = self.winner(
            DetectionCheck::Conditional,
            input.carried.conditional.as_ref(),
            check_conditional_categories(input.conditional_text, &[], self.categories)?,
        )?;
        let phrase = self.winner(
            DetectionCheck::Phrase,
            input.carried.phrase.as_ref(),
            check_phrase_patterns(input.conditional_text, &[], self.categories)?,
        )?;
        let keyword = self.winner(
            DetectionCheck::CategoryKeyword,
            input.carried.keyword.as_ref(),
            check_category_keywords(input.text, &[], self.categories)?,
        )?;
        Ok(CarriedMatches {
            conditional,
            phrase,
            keyword,
        })
    }

    // The check whose first match the scan keeps, with a dropped phrase suppressing the first two
    fn winner(
        &mut self,
        check: DetectionCheck,
        carried: Option<&CategoryMatch>,
        scanned: Option<CategoryMatch>,
    ) -> Result<Option<CategoryMatch>, ContentFilterActionError> {
        let Some(winner) = earlier_match(carried, scanned, check, self.categories) else {
            return Ok(None);
        };
        let mut plain = Vec::new();
        let masked = if check == DetectionCheck::CategoryKeyword {
            handle_category_keyword_match(
                &winner,
                self.filtered.as_str(),
                self.keyword_redaction_tag,
                Some(&mut plain),
            )
        } else {
            handle_conditional_match(&winner, self.text, Some(&mut plain))
        };
        let position = check_position(check, &winner, self.categories);
        key_plain(check, position, &mut plain, self.detections);
        *self.filtered = masked?;
        Ok(Some(winner))
    }
}

// Patterns and blocked words run over the chunk alone, so their spans keep the buffer's own offsets
fn mask_patterns_and_words(
    source: &ContentFilterSource<'_>,
    text: &str,
    detections: &mut Vec<StreamedDetection>,
) -> Result<String, ContentFilterActionError> {
    let empty_categories = CategoryLoadResult::default();
    let rest_source = source_with_categories(source, &empty_categories, true);
    let mut plain = Vec::new();
    let outcome = filter_single_text(&rest_source, text, Some(&mut plain));
    detections.extend(
        plain
            .into_iter()
            .map(|detection| rest_detection(source, detection)),
    );
    outcome
}

// The source a scan sees: the configured categories, or none while a dropped phrase suppresses them
pub(super) const fn source_with_categories<'scan>(
    source: &'scan ContentFilterSource<'_>,
    empty: &'scan CategoryLoadResult,
    suppress: bool,
) -> ContentFilterSource<'scan> {
    let categories = if suppress { empty } else { source.categories };
    ContentFilterSource {
        categories,
        patterns: source.patterns,
        blocked_words: source.blocked_words,
        pattern_redaction_format: source.pattern_redaction_format,
        keyword_redaction_tag: source.keyword_redaction_tag,
    }
}

// A kept match carries the check it came from and its place in that check's order
fn key_plain(
    check: DetectionCheck,
    position: (usize, usize),
    plain: &mut Vec<ContentFilterDetection>,
    detections: &mut Vec<StreamedDetection>,
) {
    detections.extend(plain.drain(..).map(|detection| StreamedDetection {
        check,
        position,
        detection,
    }));
}

fn rest_detection(
    source: &ContentFilterSource<'_>,
    detection: ContentFilterDetection,
) -> StreamedDetection {
    let (check, position) = match &detection {
        ContentFilterDetection::Pattern(pattern) => (
            DetectionCheck::Pattern,
            (
                0,
                source
                    .patterns
                    .iter()
                    .position(|entry| entry.pattern_name == pattern.pattern_name)
                    .unwrap_or(usize::MAX),
            ),
        ),
        ContentFilterDetection::BlockedWord(word) => (
            DetectionCheck::BlockedWord,
            (
                0,
                source
                    .blocked_words
                    .iter()
                    .position(|entry| entry.keyword == word.keyword)
                    .unwrap_or(usize::MAX),
            ),
        ),
        // the empty category set above leaves nothing else for the pattern and word scan to record
        ContentFilterDetection::CategoryKeyword(_)
        | ContentFilterDetection::CompetitorIntent(_) => (DetectionCheck::CategoryKeyword, (0, 0)),
    };
    StreamedDetection {
        check,
        position,
        detection,
    }
}

// The first match in the check's own order wins, and the dropped prefix comes first on a tie
pub(super) fn earlier_match(
    carried: Option<&CategoryMatch>,
    found: Option<CategoryMatch>,
    check: DetectionCheck,
    categories: &CategoryLoadResult,
) -> Option<CategoryMatch> {
    let Some(scanned) = found else {
        return carried.cloned();
    };
    match carried {
        Some(prefix)
            if check_position(check, prefix, categories)
                <= check_position(check, &scanned, categories) =>
        {
            Some(prefix.clone())
        }
        _ => Some(scanned),
    }
}

// Where a match sits in its check's iteration order: the category first, then the pattern in it
fn check_position(
    check: DetectionCheck,
    found: &CategoryMatch,
    categories: &CategoryLoadResult,
) -> (usize, usize) {
    let phrase = found.matched.strip_prefix("phrase: ").unwrap_or_default();
    let phrase_pattern = categories
        .loaded_categories
        .get(&found.category)
        .and_then(|config| {
            config
                .phrase_patterns
                .iter()
                .position(|pattern| pattern.source == phrase)
        })
        .unwrap_or(usize::MAX);
    let keyword = categories
        .always_block_category_keywords
        .keys()
        .chain(categories.category_keywords.keys())
        .position(|keyword| *keyword == found.matched)
        .unwrap_or(usize::MAX);
    match check {
        DetectionCheck::Conditional => (
            category_position(&categories.conditional_categories, &found.category),
            0,
        ),
        DetectionCheck::Phrase => (
            category_position(&categories.loaded_categories, &found.category),
            phrase_pattern,
        ),
        DetectionCheck::CategoryKeyword => (keyword, 0),
        DetectionCheck::Pattern | DetectionCheck::BlockedWord => (usize::MAX, usize::MAX),
    }
}

fn category_position<Entry>(categories: &BTreeMap<String, Entry>, name: &str) -> usize {
    categories
        .keys()
        .position(|category| category == name)
        .unwrap_or(usize::MAX)
}
