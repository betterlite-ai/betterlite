use super::width::{is_cjk_word_boundary, is_text_space, text_width};
use super::{CUE_GAP_MS, CUE_MAX_CHARS, CUE_MAX_DURATION_MS, SubtitleCue, SubtitleToken, Word};

const SENTENCE_END_CHARS: &str = ".!?。！？؟۔।॥։።";

#[must_use]
pub fn starts_new_word(prev: &SubtitleToken, token: &SubtitleToken) -> bool {
    let Some(first) = token.text.chars().next() else {
        return false;
    };
    let previous_last = prev.text.chars().last();
    let spaced = is_text_space(first) || previous_last.is_some_and(is_text_space);
    let boundary = previous_last.is_some_and(|last| is_cjk_word_boundary(last, first));
    spaced || token.speaker != prev.speaker || boundary
}

#[must_use]
pub fn build_word(tokens: &[&SubtitleToken]) -> Option<Word> {
    let first = tokens.first()?;
    let text = tokens
        .iter()
        .map(|token| token.text.as_str())
        .collect::<String>();
    let start_ms = tokens.iter().find_map(|token| token.start_ms);
    let end_ms = tokens.iter().rev().find_map(|token| token.end_ms);
    Some(Word {
        text,
        start_ms,
        end_ms,
        speaker: first.speaker.clone(),
    })
}

#[must_use]
pub fn merge_tokens_into_words(tokens: &[SubtitleToken]) -> Vec<Word> {
    let mut words: Vec<Word> = Vec::new();
    let mut current: Vec<&SubtitleToken> = Vec::new();
    let mut previous: Option<&SubtitleToken> = None;
    for token in tokens.iter().filter(|token| !token.text.is_empty()) {
        if previous.is_some_and(|prev| starts_new_word(prev, token)) {
            words.extend(build_word(&current));
            current.clear();
        }
        current.push(token);
        previous = Some(token);
    }
    words.extend(build_word(&current));
    words
}

#[must_use]
pub fn cue_start(words: &[Word]) -> Option<i64> {
    for word in words {
        if let Some(start) = word.start_ms {
            return Some(start);
        }
    }
    None
}

#[must_use]
pub fn cue_end(words: &[Word]) -> Option<i64> {
    words
        .iter()
        .rev()
        .find_map(|word| word.end_ms)
        .or_else(|| cue_start(words))
}

#[must_use]
pub fn cue_text(words: &[Word]) -> String {
    words
        .iter()
        .map(|word| word.text.as_str())
        .collect::<String>()
        .trim_matches(is_text_space)
        .to_owned()
}

#[must_use]
pub fn should_break(cue: &[Word], word: &Word) -> bool {
    let speaker_changed = word.speaker.is_some()
        && cue
            .iter()
            .any(|earlier| earlier.speaker.is_some() && earlier.speaker != word.speaker);
    let cue_finish = cue_end(cue);
    let gap_exceeded = word
        .start_ms
        .is_some_and(|start| cue_finish.is_some_and(|end| start.saturating_sub(end) >= CUE_GAP_MS));
    let chars_exceeded =
        text_width(&cue_text(cue)).saturating_add(text_width(&word.text)) > CUE_MAX_CHARS;
    let word_end = word.end_ms.or(word.start_ms);
    let duration_exceeded = word_end.is_some_and(|end| {
        cue_start(cue).is_some_and(|start| end.saturating_sub(start) > CUE_MAX_DURATION_MS)
    });
    speaker_changed || gap_exceeded || chars_exceeded || duration_exceeded
}

#[must_use]
pub fn cue_start_indices(words: &[Word]) -> Vec<usize> {
    let mut starts: Vec<usize> = Vec::new();
    if words.is_empty() {
        return starts;
    }
    starts.push(0);
    let mut start = 0;
    let mut previous: Option<&Word> = None;
    for (index, word) in words.iter().enumerate() {
        if let Some(previous_word) = previous {
            let sentence_end = previous_word
                .text
                .trim_end_matches(is_text_space)
                .chars()
                .last()
                .is_some_and(|last| SENTENCE_END_CHARS.contains(last));
            if sentence_end || should_break(words.get(start..index).unwrap_or_default(), word) {
                start = index;
                starts.push(index);
            }
        }
        previous = Some(word);
    }
    starts
}

#[must_use]
pub fn build_cue(words: &[Word]) -> Option<SubtitleCue> {
    let text = cue_text(words);
    let start = cue_start(words)?;
    if text.is_empty() {
        return None;
    }
    let end = cue_end(words).unwrap_or(start);
    Some(SubtitleCue {
        start_ms: start,
        end_ms: end,
        text,
    })
}

#[must_use]
pub fn group_tokens_into_cues(tokens: &[SubtitleToken]) -> Vec<SubtitleCue> {
    let words = merge_tokens_into_words(tokens);
    let starts = cue_start_indices(&words);
    let mut cues: Vec<SubtitleCue> = Vec::new();
    for (position, begin) in starts.iter().enumerate() {
        let end = starts
            .get(position.saturating_add(1))
            .copied()
            .unwrap_or(words.len());
        if let Some(cue) = words.get(*begin..end).and_then(build_cue) {
            cues.push(cue);
        }
    }
    cues
}
