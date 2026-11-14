#![cfg(test)]
#![cfg_attr(
    test,
    expect(
        clippy::default_numeric_fallback,
        clippy::disallowed_methods,
        clippy::non_ascii_literal,
        reason = "json! fixtures and CJK and RTL fixtures state the input text directly"
    )
)]

use rstest::rstest;
use serde_json::json;

use super::grouping::{
    build_word, cue_end, cue_start, group_tokens_into_cues, merge_tokens_into_words,
};
use super::render::{
    format_timestamp, render_subtitle_tokens_as_srt, render_subtitle_tokens_as_vtt,
};
use super::timing::{
    SubtitleTimingError, parse_word_timings, seconds_to_ms, synthesize_subtitle_document,
    word_to_subtitle_token,
};
use super::width::{is_cjk, is_cjk_word_boundary, text_width};
use super::{
    CUE_GAP_MS, CUE_MAX_CHARS, CUE_MAX_DURATION_MS, Speaker, SubtitleCue, SubtitleToken, Word,
};

fn timed(text: &str, start_ms: i64, end_ms: i64) -> SubtitleToken {
    SubtitleToken {
        text: text.to_owned(),
        start_ms: Some(start_ms),
        end_ms: Some(end_ms),
        speaker: None,
    }
}

fn spoken(text: &str, start_ms: i64, end_ms: i64, speaker: &str) -> SubtitleToken {
    let mut token = timed(text, start_ms, end_ms);
    token.speaker = Some(Speaker::Text(speaker.to_owned()));
    token
}

fn cue(start_ms: i64, end_ms: i64, text: &str) -> SubtitleCue {
    SubtitleCue {
        start_ms,
        end_ms,
        text: text.to_owned(),
    }
}

fn texts(words: &[Word]) -> Vec<&str> {
    words.iter().map(|word| word.text.as_str()).collect()
}

#[test]
fn subtitle_rendering_merges_subword_tokens_into_one_cue() {
    let tokens = [
        SubtitleToken {
            text: "Hell".to_owned(),
            start_ms: Some(0),
            end_ms: None,
            speaker: None,
        },
        SubtitleToken {
            text: "o".to_owned(),
            start_ms: None,
            end_ms: Some(1000),
            speaker: None,
        },
    ];
    assert_eq!(group_tokens_into_cues(&tokens), vec![cue(0, 1000, "Hello")]);
    assert_eq!(
        render_subtitle_tokens_as_srt(&tokens),
        "1\n00:00:00,000 --> 00:00:01,000\nHello\n"
    );
    assert_eq!(
        render_subtitle_tokens_as_vtt(&tokens),
        "WEBVTT\n\n00:00:00.000 --> 00:00:01.000\nHello\n"
    );
}

#[test]
fn subtitle_rendering_empty_tokens_render_empty_srt_and_header_only_vtt() {
    assert_eq!(render_subtitle_tokens_as_srt(&[]), "");
    assert_eq!(render_subtitle_tokens_as_vtt(&[]), "WEBVTT\n");
}

#[test]
fn subtitle_rendering_tokens_without_a_start_render_no_cue() {
    let tokens = [SubtitleToken {
        text: "Hi".to_owned(),
        start_ms: None,
        end_ms: Some(100),
        speaker: None,
    }];
    assert_eq!(render_subtitle_tokens_as_srt(&tokens), "");
    assert_eq!(render_subtitle_tokens_as_vtt(&tokens), "WEBVTT\n");
    assert!(
        synthesize_subtitle_document(&json!([{"word": "Hi", "end": 0.1}]), "srt")
            .expect("a valid timing")
            .is_none()
    );
}

#[test]
fn subtitle_rendering_negative_cues_render_zero_without_changing_the_cue() {
    let tokens = [timed("X", -1, -2)];
    assert_eq!(group_tokens_into_cues(&tokens), vec![cue(-1, -2, "X")]);
    assert_eq!(
        render_subtitle_tokens_as_srt(&tokens),
        "1\n00:00:00,000 --> 00:00:00,000\nX\n"
    );
    assert_eq!(format_timestamp(-1, '.'), "00:00:00.000");
}

#[test]
fn subtitle_rendering_missing_end_uses_the_start_and_keeps_an_earlier_end() {
    let missing = [timed("X", 5, 5)];
    let earlier = [SubtitleToken {
        text: "X".to_owned(),
        start_ms: Some(5),
        end_ms: Some(2),
        speaker: None,
    }];
    assert_eq!(group_tokens_into_cues(&missing), vec![cue(5, 5, "X")]);
    assert_eq!(group_tokens_into_cues(&earlier), vec![cue(5, 2, "X")]);
    assert_eq!(
        render_subtitle_tokens_as_srt(&earlier),
        "1\n00:00:00,005 --> 00:00:00,002\nX\n"
    );
}

#[test]
fn subtitle_rendering_hours_may_exceed_two_digits() {
    let tokens = [timed("X", 360_000_000, 360_000_000)];
    assert_eq!(
        render_subtitle_tokens_as_srt(&tokens),
        "1\n100:00:00,000 --> 100:00:00,000\nX\n"
    );
}

#[test]
fn subtitle_rendering_numbers_cues_and_keeps_a_blank_line_between_them() {
    let tokens = [
        timed("A. ", 0, 100),
        SubtitleToken {
            text: " B".to_owned(),
            start_ms: Some(200),
            end_ms: Some(300),
            speaker: None,
        },
        SubtitleToken {
            text: "C ".to_owned(),
            start_ms: Some(400),
            end_ms: Some(500),
            speaker: None,
        },
        timed("D", 600, 700),
    ];
    assert_eq!(
        group_tokens_into_cues(&tokens),
        vec![cue(0, 100, "A."), cue(200, 700, "BC D")]
    );
    assert_eq!(
        render_subtitle_tokens_as_srt(&tokens),
        "1\n00:00:00,000 --> 00:00:00,100\nA.\n\n2\n00:00:00,200 --> 00:00:00,700\nBC D\n"
    );
}

#[test]
fn subtitle_rendering_trims_text_only_at_cue_construction() {
    let tokens = [timed("  Hi  ", 0, 100)];
    assert_eq!(texts(&merge_tokens_into_words(&tokens)), vec!["  Hi  "]);
    assert_eq!(group_tokens_into_cues(&tokens), vec![cue(0, 100, "Hi")]);
}

#[test]
fn subtitle_rendering_drops_words_without_any_timestamp_from_the_cue() {
    let tokens = [
        timed("Hi", 0, 100),
        SubtitleToken {
            text: " there".to_owned(),
            start_ms: None,
            end_ms: None,
            speaker: None,
        },
        SubtitleToken {
            text: String::new(),
            start_ms: Some(999),
            end_ms: None,
            speaker: None,
        },
    ];
    assert_eq!(
        group_tokens_into_cues(&tokens),
        vec![cue(0, 100, "Hi there")]
    );
}

#[test]
fn subtitle_thresholds_gap_below_the_limit_keeps_one_cue() {
    let tokens = [timed("a ", 0, 1000), timed("b", 1699, 2000)];
    assert_eq!(group_tokens_into_cues(&tokens), vec![cue(0, 2000, "a b")]);
    assert_eq!(CUE_GAP_MS, 700);
}

#[test]
fn subtitle_thresholds_gap_at_the_limit_breaks_the_cue() {
    let tokens = [timed("a ", 0, 1000), timed("b", 1700, 2000)];
    assert_eq!(
        group_tokens_into_cues(&tokens),
        vec![cue(0, 1000, "a"), cue(1700, 2000, "b")]
    );
}

#[test]
fn subtitle_thresholds_duration_at_the_limit_keeps_one_cue() {
    let tokens = [timed("a ", 0, 1000), timed("b", 1000, 7000)];
    assert_eq!(group_tokens_into_cues(&tokens), vec![cue(0, 7000, "a b")]);
    assert_eq!(CUE_MAX_DURATION_MS, 7000);
}

#[test]
fn subtitle_thresholds_duration_past_the_limit_breaks_the_cue() {
    let tokens = [timed("a ", 0, 1000), timed("b", 1000, 7001)];
    assert_eq!(
        group_tokens_into_cues(&tokens),
        vec![cue(0, 1000, "a"), cue(1000, 7001, "b")]
    );
}

#[test]
fn subtitle_thresholds_width_at_the_limit_keeps_one_cue() {
    let tokens = [
        timed(&format!("{} ", "a".repeat(42)), 0, 100),
        timed(&"b".repeat(42), 100, 200),
    ];
    assert_eq!(
        group_tokens_into_cues(&tokens),
        vec![cue(
            0,
            200,
            &format!("{} {}", "a".repeat(42), "b".repeat(42))
        )]
    );
    assert_eq!(CUE_MAX_CHARS, 84);
}

#[test]
fn subtitle_thresholds_width_past_the_limit_breaks_at_the_word_boundary() {
    let tokens = [
        timed(&format!("{} ", "a".repeat(42)), 0, 100),
        timed(&"b".repeat(43), 100, 200),
    ];
    assert_eq!(
        group_tokens_into_cues(&tokens),
        vec![cue(0, 100, &"a".repeat(42)), cue(100, 200, &"b".repeat(43)),]
    );
}

#[test]
fn subtitle_thresholds_keep_a_single_overlong_word_whole() {
    let tokens = [timed(&"a".repeat(200), 0, 100)];
    assert_eq!(
        group_tokens_into_cues(&tokens),
        vec![cue(0, 100, &"a".repeat(200))]
    );
}

#[test]
fn subtitle_thresholds_a_known_speaker_change_breaks_the_cue() {
    let tokens = [spoken("a ", 0, 100, "a"), spoken("b", 100, 200, "b")];
    assert_eq!(
        group_tokens_into_cues(&tokens),
        vec![cue(0, 100, "a"), cue(100, 200, "b")]
    );
}

#[test]
fn subtitle_thresholds_a_known_speaker_followed_by_unknown_does_not_break() {
    let tokens = [spoken("a ", 0, 100, "a"), timed("b", 100, 200)];
    assert_eq!(group_tokens_into_cues(&tokens), vec![cue(0, 200, "a b")]);
}

#[test]
fn subtitle_thresholds_a_later_known_speaker_breaks_after_unknown_words() {
    let tokens = [
        spoken("a ", 0, 100, "a"),
        timed("b ", 100, 200),
        spoken("c", 200, 300, "b"),
    ];
    assert_eq!(
        group_tokens_into_cues(&tokens),
        vec![cue(0, 200, "a b"), cue(200, 300, "c")]
    );
}

#[test]
fn subtitle_thresholds_merge_subtokens_into_one_overlong_word() {
    let tokens = vec![timed("a", 0, 100); 100];
    assert_eq!(
        group_tokens_into_cues(&tokens),
        vec![cue(0, 100, &"a".repeat(100))]
    );
}

#[test]
fn subtitle_thresholds_a_leading_space_starts_a_new_word() {
    let tokens = [timed(&"a".repeat(84), 0, 100), timed(" b", 100, 200)];
    assert_eq!(
        group_tokens_into_cues(&tokens),
        vec![cue(0, 100, &"a".repeat(84)), cue(100, 200, "b")]
    );
}

#[test]
fn subtitle_thresholds_a_known_speaker_change_without_spaces_starts_a_new_word() {
    let tokens = [
        spoken(&"a".repeat(84), 0, 100, "a"),
        spoken("b", 100, 200, "b"),
    ];
    assert_eq!(
        group_tokens_into_cues(&tokens),
        vec![cue(0, 100, &"a".repeat(84)), cue(100, 200, "b")]
    );
}

#[test]
fn subtitle_thresholds_a_known_speaker_after_only_unknown_ones_does_not_break() {
    let tokens = [timed("a ", 0, 100), spoken("b", 100, 200, "b")];
    assert_eq!(group_tokens_into_cues(&tokens), vec![cue(0, 200, "a b")]);
}

#[rstest]
#[case::period(". ")]
#[case::exclamation("! ")]
#[case::question("? ")]
#[case::ideographic_full_stop("。 ")]
#[case::fullwidth_exclamation("！ ")]
#[case::fullwidth_question("？ ")]
#[case::arabic_question("؟ ")]
#[case::arabic_full_stop("۔ ")]
#[case::devanagari_danda("। ")]
#[case::devanagari_double_danda("॥ ")]
#[case::armenian_full_stop("։ ")]
#[case::ethiopic_full_stop("። ")]
fn subtitle_thresholds_sentence_endings_break_before_the_next_word(#[case] ending: &str) {
    let tokens = [timed(ending, 0, 100), timed("b", 100, 200)];
    assert_eq!(
        group_tokens_into_cues(&tokens),
        vec![cue(0, 100, ending.trim_end()), cue(100, 200, "b")]
    );
}

#[rstest]
#[case::extension_b(('\u{3400}', '\u{4DBF}'), [('\u{33FF}', false), ('\u{4DC0}', false)])]
#[case::unified_ideographs(('\u{4E00}', '\u{9FFF}'), [('\u{4DFF}', false), ('\u{A000}', false)])]
#[case::compatibility_ideographs(('\u{F900}', '\u{FAFF}'), [('\u{F8FF}', false), ('\u{FB00}', false)])]
#[case::hiragana(('\u{3040}', '\u{309F}'), [('\u{303F}', false), ('\u{30A0}', true)])]
#[case::katakana(('\u{30A0}', '\u{30FF}'), [('\u{309F}', true), ('\u{3100}', false)])]
#[case::katakana_phonetic(('\u{31F0}', '\u{31FF}'), [('\u{31EF}', false), ('\u{3200}', false)])]
fn subtitle_cjk_boundaries_cover_every_range_endpoint(
    #[case] range: (char, char),
    #[case] neighbors: [(char, bool); 2],
) {
    assert!(is_cjk(range.0));
    assert!(is_cjk(range.1));
    for (neighbor, is_member) in neighbors {
        assert_eq!(is_cjk(neighbor), is_member, "U+{:04X}", u32::from(neighbor));
    }
}

#[test]
fn subtitle_cjk_boundaries_make_character_pairs_words_without_spaces() {
    let tokens = [timed("你", 0, 100), timed("好", 100, 200)];
    assert_eq!(texts(&merge_tokens_into_words(&tokens)), vec!["你", "好"]);
    assert_eq!(group_tokens_into_cues(&tokens), vec![cue(0, 200, "你好")]);
    let japanese = [timed("あ", 0, 100), timed("い", 100, 200)];
    assert_eq!(texts(&merge_tokens_into_words(&japanese)), vec!["あ", "い"]);
}

#[test]
fn subtitle_cjk_boundaries_keep_kinsoku_punctuation_attached() {
    let closing = [
        timed("你", 0, 100),
        timed("好", 100, 200),
        timed("，", 200, 300),
        timed("世", 300, 400),
        timed("界", 400, 500),
    ];
    assert_eq!(
        texts(&merge_tokens_into_words(&closing)),
        vec!["你", "好，", "世", "界"]
    );
    let opening = [
        timed("「", 0, 100),
        timed("你", 100, 200),
        timed("好", 200, 300),
        timed("」", 300, 400),
    ];
    assert_eq!(
        texts(&merge_tokens_into_words(&opening)),
        vec!["「你", "好」"]
    );
}

#[test]
fn subtitle_cjk_boundaries_apply_kinsoku_only_across_a_cjk_character() {
    assert!(is_cjk_word_boundary('你', '好'));
    assert!(is_cjk_word_boundary('a', '你'));
    assert!(!is_cjk_word_boundary('a', 'b'));
    assert!(!is_cjk_word_boundary('你', '，'));
    assert!(!is_cjk_word_boundary('「', '你'));
    assert!(!is_cjk_word_boundary('你', '」'));
    assert!(!is_cjk_word_boundary('a', '。'));
}

#[rstest]
#[case::fullwidth_latin('Ａ', 2)]
#[case::ideograph('一', 2)]
#[case::fullwidth_comma('，', 2)]
#[case::narrow('a', 1)]
#[case::combining_mark('\u{301}', 1)]
#[case::ambiguous_arrow('→', 1)]
#[case::fullwidth_space('\u{3000}', 2)]
fn subtitle_cjk_boundaries_count_display_width(#[case] character: char, #[case] width: usize) {
    assert_eq!(text_width(&character.to_string()), width);
    assert_eq!(
        text_width(&format!("a{character}b")),
        width.saturating_add(2)
    );
}

#[test]
fn subtitle_cjk_boundaries_split_where_display_width_exceeds_the_limit() {
    let prefix = "一".repeat(41);
    let tokens = [
        timed(&prefix, 0, 100),
        timed("㐀", 100, 200),
        timed("䷀", 200, 300),
    ];
    assert_eq!(
        group_tokens_into_cues(&tokens),
        vec![cue(0, 200, &format!("{prefix}㐀")), cue(200, 300, "䷀"),]
    );
}

#[test]
fn subtitle_cjk_boundaries_count_widths_through_cue_splits() {
    let fullwidth = [
        timed(&format!("{} ", "Ａ".repeat(42)), 0, 100),
        timed("b", 100, 200),
    ];
    assert_eq!(
        group_tokens_into_cues(&fullwidth),
        vec![cue(0, 100, &"Ａ".repeat(42)), cue(100, 200, "b"),]
    );
    let combining = [
        timed(&format!("{} ", "\u{61}\u{301}".repeat(42)), 0, 100),
        timed("b", 100, 200),
    ];
    assert_eq!(group_tokens_into_cues(&combining).len(), 2);
    let narrow = [
        timed(&format!("{} ", "a".repeat(42)), 0, 100),
        timed("b", 100, 200),
    ];
    assert_eq!(group_tokens_into_cues(&narrow).len(), 1);
}

#[test]
fn subtitle_cjk_boundaries_drop_empty_tokens_and_keep_outer_timestamps() {
    let tokens = [
        SubtitleToken {
            text: "Hell".to_owned(),
            start_ms: Some(5),
            end_ms: None,
            speaker: None,
        },
        SubtitleToken {
            text: String::new(),
            start_ms: Some(7),
            end_ms: Some(8),
            speaker: None,
        },
        SubtitleToken {
            text: "o".to_owned(),
            start_ms: None,
            end_ms: Some(9),
            speaker: None,
        },
    ];
    assert_eq!(texts(&merge_tokens_into_words(&tokens)), vec!["Hello"]);
    assert_eq!(group_tokens_into_cues(&tokens), vec![cue(5, 9, "Hello")]);
}

#[test]
fn subtitle_cjk_boundaries_ignore_empty_tokens_between_words() {
    let prefix = "一".repeat(41);
    let tokens = [
        timed(&prefix, 0, 100),
        timed("㐀", 100, 200),
        SubtitleToken {
            text: String::new(),
            start_ms: Some(200),
            end_ms: Some(250),
            speaker: None,
        },
        timed("䷀", 250, 300),
    ];
    assert_eq!(
        group_tokens_into_cues(&tokens),
        vec![cue(0, 200, &format!("{prefix}㐀")), cue(250, 300, "䷀")]
    );
}

#[test]
fn subtitle_cjk_boundaries_invent_no_timestamp_for_a_missing_start() {
    let tokens = [
        SubtitleToken {
            text: "Hell".to_owned(),
            start_ms: None,
            end_ms: None,
            speaker: None,
        },
        timed("o", 3, 3),
    ];
    assert_eq!(group_tokens_into_cues(&tokens), vec![cue(3, 3, "Hello")]);
}

#[test]
fn subtitle_word_timing_input_renders_srt_from_word_records() {
    let words = json!([{"word": "Hello", "start": 0, "end": 1, "extra": {"ignored": true}}]);
    assert_eq!(
        synthesize_subtitle_document(&words, "srt").expect("a valid timing"),
        Some("1\n00:00:00,000 --> 00:00:01,000\nHello\n".to_owned())
    );
}

#[test]
fn subtitle_word_timing_input_renders_vtt_from_word_records() {
    let words = json!([{"word": "Hello", "start": 0, "end": 1}]);
    assert_eq!(
        synthesize_subtitle_document(&words, "vtt").expect("a valid timing"),
        Some("WEBVTT\n\n00:00:00.000 --> 00:00:01.000\nHello\n".to_owned())
    );
}

#[rstest]
#[case::json_format("json")]
#[case::uppercase_srt("SRT")]
#[case::trailing_space("srt ")]
fn subtitle_word_timing_input_rejects_unsupported_formats(#[case] response_format: &str) {
    let words = json!([{"word": "Hello", "start": 0, "end": 1}]);
    assert_eq!(
        synthesize_subtitle_document(&words, response_format).expect("a valid timing"),
        None
    );
}

#[test]
fn subtitle_word_timing_input_coerces_numeric_strings() {
    let words = json!([{"word": "Hi", "start": "0.5", "end": "1.25"}]);
    assert_eq!(
        synthesize_subtitle_document(&words, "srt").expect("a valid timing"),
        Some("1\n00:00:00,500 --> 00:00:01,250\nHi\n".to_owned())
    );
}

#[rstest]
#[case::unparsable_start(json!([{"word": "Hi", "start": "bad", "end": 1}]))]
#[case::numeric_speaker(json!([{"word": "Hi", "start": 0, "speaker": 7}]))]
#[case::missing_record_list(json!({"word": "Hi", "start": 0}))]
#[case::bad_record_after_good(json!([{"word": "Hi", "start": 0}, {"word": "Ho", "start": "bad"}]))]
fn subtitle_word_timing_input_drops_the_whole_list_silently(#[case] words: serde_json::Value) {
    assert!(parse_word_timings(&words).is_empty());
    assert_eq!(
        synthesize_subtitle_document(&words, "srt").expect("no timing error"),
        None
    );
}

#[test]
fn subtitle_word_timing_input_defaults_a_missing_word_to_a_whitespace_token() {
    let words = json!([{"start": 0, "end": 1}]);
    let parsed = parse_word_timings(&words);
    assert_eq!(parsed.len(), 1);
    assert_eq!(parsed[0].word, "");
    assert_eq!(
        synthesize_subtitle_document(&words, "srt").expect("a valid timing"),
        None
    );
}

#[rstest]
#[case::exact_half_below(0.0005, 0)]
#[case::one_and_a_half(0.0015, 2)]
#[case::two_and_a_half(0.0025, 2)]
#[case::three_and_a_half(0.0035, 4)]
fn subtitle_word_timing_input_rounds_milliseconds_to_even(
    #[case] seconds: f64,
    #[case] millis: i64,
) {
    assert_eq!(
        seconds_to_ms(Some(seconds)).expect("a valid time"),
        Some(millis)
    );
}

#[test]
fn subtitle_word_timing_input_accepts_booleans_and_nulls_as_pydantic_does() {
    let words = json!([{"word": "Hi", "start": true, "end": false}]);
    assert_eq!(
        synthesize_subtitle_document(&words, "srt").expect("a valid timing"),
        Some("1\n00:00:01,000 --> 00:00:00,000\nHi\n".to_owned())
    );
    assert_eq!(
        parse_word_timings(&json!([{"word": "Hi", "start": null}]))[0].start,
        None
    );
}

#[test]
fn subtitle_word_timing_input_fails_on_a_nan_timing() {
    assert_eq!(
        seconds_to_ms(Some(f64::NAN)),
        Err(SubtitleTimingError::NotANumber)
    );
    let words = json!([{"word": "Hi", "start": "nan", "end": 1}]);
    assert_eq!(
        synthesize_subtitle_document(&words, "srt"),
        Err(SubtitleTimingError::NotANumber)
    );
}

#[test]
fn subtitle_word_timing_input_fails_on_an_infinite_timing() {
    assert_eq!(
        seconds_to_ms(Some(f64::INFINITY)),
        Err(SubtitleTimingError::OutOfRange)
    );
    assert_eq!(
        seconds_to_ms(Some(f64::NEG_INFINITY)),
        Err(SubtitleTimingError::OutOfRange)
    );
    assert_eq!(
        seconds_to_ms(Some(1e308)),
        Err(SubtitleTimingError::OutOfRange)
    );
    let words = json!([{"word": "Hi", "start": "inf", "end": 1}]);
    assert_eq!(
        synthesize_subtitle_document(&words, "srt"),
        Err(SubtitleTimingError::OutOfRange)
    );
}

#[test]
fn subtitle_word_timing_input_checks_the_format_before_the_timings() {
    let words = json!([{"word": "Hi", "start": "nan"}]);
    assert_eq!(synthesize_subtitle_document(&words, "json"), Ok(None));
}

#[test]
fn subtitle_word_timing_input_keeps_an_end_before_the_start() {
    let words = json!([{"word": "X", "start": 1, "end": 0}]);
    assert_eq!(
        synthesize_subtitle_document(&words, "srt").expect("a valid timing"),
        Some("1\n00:00:01,000 --> 00:00:00,000\nX\n".to_owned())
    );
}

#[test]
fn subtitle_word_timing_input_merges_words_that_carry_a_trailing_space() {
    let words = json!([
        {"word": "Hello", "start": 0, "end": 0.5},
        {"word": "world", "start": 0.6, "end": 1}
    ]);
    assert_eq!(
        synthesize_subtitle_document(&words, "srt").expect("a valid timing"),
        Some("1\n00:00:00,000 --> 00:00:01,000\nHello world\n".to_owned())
    );
}

#[test]
fn subtitle_word_timing_input_turns_a_record_into_a_spaced_token() {
    let word = parse_word_timings(&json!([{"word": "Hi", "start": 0.5, "speaker": "a"}]));
    let token = word_to_subtitle_token(&word[0]).expect("a valid timing");
    assert_eq!(token.text, "Hi ");
    assert_eq!(token.start_ms, Some(500));
    assert_eq!(token.end_ms, None);
    assert_eq!(token.speaker, Some(Speaker::Text("a".to_owned())));
}

#[test]
fn subtitle_rendering_builds_words_with_the_first_start_and_last_end() {
    let tokens = [
        SubtitleToken {
            text: "Hell".to_owned(),
            start_ms: Some(5),
            end_ms: Some(6),
            speaker: Some(Speaker::Text("a".to_owned())),
        },
        SubtitleToken {
            text: "o".to_owned(),
            start_ms: Some(7),
            end_ms: Some(9),
            speaker: Some(Speaker::Number(2)),
        },
    ];
    let word = build_word(&[&tokens[0], &tokens[1]]).expect("a word");
    assert_eq!(word.text, "Hello");
    assert_eq!(word.start_ms, Some(5));
    assert_eq!(word.end_ms, Some(9));
    assert_eq!(word.speaker, Some(Speaker::Text("a".to_owned())));
}

#[test]
fn subtitle_rendering_falls_back_from_a_missing_end_to_the_start() {
    let words = [Word {
        text: "X".to_owned(),
        start_ms: Some(4),
        end_ms: None,
        speaker: None,
    }];
    assert_eq!(cue_start(&words), Some(4));
    assert_eq!(cue_end(&words), Some(4));
    let empty: [Word; 0] = [];
    assert_eq!(cue_start(&empty), None);
    assert_eq!(cue_end(&empty), None);
}
