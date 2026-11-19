use betterlite_config::sections::settings::JsonValue;
use serde_json::json;

use super::support::{envelope, expect_result, indexed, json_value, results};

pub(super) fn citation_snippets() {
    filters_non_citations();
    snippets_follow_markers();
    start_index_boundaries();
    empty_claim_falls_back();
    long_claims_keep_the_tail();
    unicode_indices_count_characters();
    duplicates_keep_first();
}

fn filters_non_citations() {
    let body = envelope(
        r#"[
            {"type":"reasoning","content":[{"type":"output_text","text":"hidden","annotations":[{"type":"url_citation","url":"https://hidden.example"}]}]},
            {"type":"message","content":[{"type":"text","text":"ignored","annotations":[{"type":"url_citation","url":"https://text.example"}]}]},
            {"type":"message","content":[{"type":"output_text","text":"plain","annotations":[
                {"type":"file_citation","url":"https://file.example"},
                {"type":"url_citation","url":""},
                {"type":"url_citation"},
                {"type":"url_citation","url":"https://kept.example","title":"Kept"}
            ]}]}
        ]"#,
    );
    assert_eq!(
        results(&body),
        vec![expect_result("Kept", "https://kept.example", "plain")]
    );
}

fn snippets_follow_markers() {
    let text = "First claim ([a](https://a.example)).\nSecond claim ([b](https://b.example)).";
    let quoted = serde_json::to_string(text).unwrap();
    let first = text.find("([a]").unwrap();
    let second = text.find("([b]").unwrap();
    let body = envelope(&format!(
        r#"[{{"type":"message","content":[{{"type":"output_text","text":{quoted},"annotations":[
            {{"type":"url_citation","url":"https://a.example","title":"A","start_index":{first}}},
            {{"type":"url_citation","url":"https://b.example","start_index":{second}}}
        ]}}]}}]"#,
    ));
    assert_eq!(
        results(&body),
        vec![
            expect_result("A", "https://a.example", "First claim"),
            expect_result("", "https://b.example", "Second claim"),
        ]
    );
}

fn whole_text_at(index: &JsonValue) {
    let text = "Only claim here.";
    assert_eq!(
        results(&indexed(text, index)),
        vec![expect_result("", "https://x.example", text)]
    );
}

fn start_index_boundaries() {
    let length = "Only claim here.".len();
    whole_text_at(&json!(length));
    whole_text_at(&json!(length.saturating_add(5)));
    whole_text_at(&json!(-1_i64));
    whole_text_at(&json!(null));
    whole_text_at(&json!(0_usize));
    whole_text_at(&json_value("18446744073709551616"));
    whole_text_at(&json_value("9223372036854775808"));
    let long = format!("{}([l](https://l.example))", "y".repeat(350));
    let quoted = serde_json::to_string(&long).unwrap();
    let negative_zero = envelope(&format!(
        r#"[{{"type":"message","content":[{{"type":"output_text","text":{quoted},"annotations":[{{"type":"url_citation","url":"https://x.example","start_index":-0}}]}}]}}]"#,
    ));
    assert_eq!(
        results(&negative_zero),
        vec![expect_result("", "https://x.example", &"y".repeat(300))]
    );
}

fn empty_claim_falls_back() {
    let long = "y".repeat(350);
    let long_results = results(&indexed(&long, &json!(0_usize)));
    assert_eq!(long_results.len(), 1_usize);
    assert_eq!(long_results[0].snippet, "y".repeat(300));
    let exact = "z".repeat(300);
    let exact_results = results(&indexed(&exact, &json!(0_usize)));
    assert_eq!(exact_results[0].snippet, exact);
    let whitespace = indexed("   ([m](https://m.example))", &json!(3_usize));
    let whitespace_results = results(&whitespace);
    assert_eq!(whitespace_results[0].snippet, "   ([m](https://m.example))");
}

fn long_claims_keep_the_tail() {
    let claim = format!("{} tail", "x".repeat(350));
    let text = format!("{claim}([x](https://x.example))");
    let marker = text.find("([x]").unwrap();
    let captured = results(&indexed(&text, &json!(marker)));
    let tail: String = claim
        .chars()
        .skip(claim.chars().count().saturating_sub(300))
        .collect();
    assert_eq!(captured[0].snippet, tail);
    assert_eq!(captured[0].snippet.chars().count(), 300);
}

fn unicode_indices_count_characters() {
    let text = "\u{e9}([u](https://u.example))";
    let marker_bytes = text.find("([u]").unwrap();
    let marker_chars = text.get(..marker_bytes).unwrap().chars().count();
    let body = indexed(text, &json!(marker_chars));
    assert_eq!(
        results(&body),
        vec![expect_result("", "https://x.example", "\u{e9}")]
    );
    let claim = "\u{e9}".repeat(350);
    let long = format!("{claim}([e](https://e.example))");
    let long_bytes = long.find("([e]").unwrap();
    let long_chars = long.get(..long_bytes).unwrap().chars().count();
    let long_results = results(&indexed(&long, &json!(long_chars)));
    assert_eq!(long_results[0].snippet, "\u{e9}".repeat(300));
}

fn duplicates_keep_first() {
    let body = envelope(
        r#"[
            {"type":"message","content":[{"type":"output_text","text":"alpha","annotations":[
                {"type":"url_citation","url":"https://a.example","title":"first"},
                {"type":"url_citation","url":"https://b.example"},
                {"type":"url_citation","url":"https://a.example","title":"second"}
            ]}]}
        ]"#,
    );
    assert_eq!(
        results(&body),
        vec![
            expect_result("first", "https://a.example", "alpha"),
            expect_result("", "https://b.example", "alpha"),
        ]
    );
}
