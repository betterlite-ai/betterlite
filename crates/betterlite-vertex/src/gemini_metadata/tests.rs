#![cfg(test)]

use betterlite_models::decisions::Json;
use rstest::rstest;

use crate::gemini_metadata::annotations::{
    AnnotationType, GroundingAnnotation, UrlCitation, grounding_annotations,
};
use crate::gemini_metadata::extension::{MetadataExtension, MetadataField};
use crate::gemini_metadata::extract::extract_candidate_metadata;
use crate::gemini_metadata::stream::{assemble_stream_metadata, stream_chunk_attr};
use crate::provider_metadata::redaction::VERTEX_METADATA_FIELDS;

fn json(source: &str) -> Json {
    serde_json::from_str(source).unwrap()
}

fn extension_with(grounding: Vec<Json>) -> MetadataExtension {
    MetadataExtension {
        grounding_metadata: grounding,
        url_context_metadata: Vec::new(),
        safety_ratings: Vec::new(),
        safety_results: Vec::new(),
        citation_metadata: Vec::new(),
    }
}

fn all_fields(extension: &MetadataExtension) -> Vec<(&'static str, &[Json])> {
    MetadataField::ALL
        .into_iter()
        .map(|field| (field.wire_name(), extension.values(field)))
        .collect()
}

fn chunk(field: &str, value: &str) -> Json {
    json(&format!(r#"{{"{field}": {value}}}"#))
}

#[test]
fn extraction_wraps_a_scalar_grounding_metadata_and_flattens_a_list() {
    let scalar = extract_candidate_metadata(&json(
        r#"{"groundingMetadata": {"webSearchQueries": ["a"]}}"#,
    ));
    assert_eq!(
        scalar.grounding_metadata,
        vec![json(r#"{"webSearchQueries": ["a"]}"#)]
    );
    let listed = extract_candidate_metadata(&json(
        r#"{"groundingMetadata": [{"n": 1}, {"n": 2}, {"n": 1}]}"#,
    ));
    assert_eq!(
        listed.grounding_metadata,
        vec![
            json(r#"{"n": 1}"#),
            json(r#"{"n": 2}"#),
            json(r#"{"n": 1}"#)
        ]
    );
    let empty = extract_candidate_metadata(&json(r#"{"groundingMetadata": []}"#));
    assert!(empty.grounding_metadata.is_empty());
}

#[test]
fn extraction_keeps_safety_ratings_nested_and_aliases_safety_results() {
    let extension = extract_candidate_metadata(&json(
        r#"{"safetyRatings": [{"category": "HARM_CATEGORY_HATE_SPEECH", "probability": "NEGLIGIBLE"}, {"category": "HARM_CATEGORY_DANGEROUS_CONTENT", "probability": "LOW"}]}"#,
    ));
    let expected = vec![json(
        r#"[{"category": "HARM_CATEGORY_HATE_SPEECH", "probability": "NEGLIGIBLE"}, {"category": "HARM_CATEGORY_DANGEROUS_CONTENT", "probability": "LOW"}]"#,
    )];
    assert_eq!(extension.safety_ratings, expected);
    assert_eq!(extension.safety_results, expected);
}

#[test]
fn an_empty_safety_rating_list_stays_one_nested_value() {
    let extension = extract_candidate_metadata(&json(r#"{"safetyRatings": []}"#));
    assert_eq!(extension.safety_ratings, vec![json("[]")]);
    assert_eq!(extension.safety_results, vec![json("[]")]);
}

#[test]
fn extraction_appends_citation_and_url_context_metadata_once() {
    let extension = extract_candidate_metadata(&json(
        r#"{"citationMetadata": {"citations": [{"uri": "https://a"}]}, "urlContextMetadata": {"urlMetadata": [{"retrievedUrl": "https://b"}]}}"#,
    ));
    assert_eq!(
        extension.citation_metadata,
        vec![json(r#"{"citations": [{"uri": "https://a"}]}"#)]
    );
    assert_eq!(
        extension.url_context_metadata,
        vec![json(r#"{"urlMetadata": [{"retrievedUrl": "https://b"}]}"#)]
    );
}

#[rstest]
#[case::missing("{}")]
#[case::null(
    r#"{"groundingMetadata": null, "safetyRatings": null, "citationMetadata": null, "urlContextMetadata": null}"#
)]
fn absent_or_null_candidate_metadata_is_empty(#[case] source: &str) {
    assert_eq!(
        extract_candidate_metadata(&json(source)),
        MetadataExtension::default()
    );
}

#[rstest]
#[case::list("[]")]
#[case::text(r#""candidate""#)]
#[case::number("1")]
#[case::null("null")]
fn a_non_object_candidate_has_no_metadata(#[case] source: &str) {
    assert_eq!(
        extract_candidate_metadata(&json(source)),
        MetadataExtension::default()
    );
}

#[rstest]
#[case::top_level(
    r#"{"vertex_ai_grounding_metadata": ["top"], "model_extra": {"vertex_ai_grounding_metadata": ["extra"]}, "_hidden_params": {"vertex_ai_grounding_metadata": ["hidden"]}}"#,
    Some(r#"["top"]"#)
)]
#[case::model_extra(
    r#"{"model_extra": {"vertex_ai_grounding_metadata": ["extra"]}, "_hidden_params": {"vertex_ai_grounding_metadata": ["hidden"]}}"#,
    Some(r#"["extra"]"#)
)]
#[case::hidden_params(
    r#"{"_hidden_params": {"vertex_ai_grounding_metadata": ["hidden"]}}"#,
    Some(r#"["hidden"]"#)
)]
#[case::null_top_level(
    r#"{"vertex_ai_grounding_metadata": null, "model_extra": {"vertex_ai_grounding_metadata": ["extra"]}}"#,
    Some(r#"["extra"]"#)
)]
#[case::null_model_extra(
    r#"{"model_extra": {"vertex_ai_grounding_metadata": null}, "_hidden_params": {"vertex_ai_grounding_metadata": ["hidden"]}}"#,
    Some(r#"["hidden"]"#)
)]
#[case::non_object_model_extra(
    r#"{"model_extra": [], "_hidden_params": {"vertex_ai_grounding_metadata": ["hidden"]}}"#,
    Some(r#"["hidden"]"#)
)]
#[case::model_extra_without_the_field(
    r#"{"model_extra": {"other": 1}, "_hidden_params": {"vertex_ai_grounding_metadata": ["hidden"]}}"#,
    Some(r#"["hidden"]"#)
)]
#[case::null_hidden(r#"{"_hidden_params": {"vertex_ai_grounding_metadata": null}}"#, None)]
#[case::non_object_hidden(r#"{"_hidden_params": "copies"}"#, None)]
#[case::absent(r#"{"content": "text"}"#, None)]
#[case::non_object_chunk(r#""chunk""#, None)]
fn stream_lookup_reads_top_level_then_model_extra_then_hidden(
    #[case] source: &str,
    #[case] expected: Option<&str>,
) {
    let chunk = json(source);
    assert_eq!(
        stream_chunk_attr(&chunk, "vertex_ai_grounding_metadata").cloned(),
        expected.map(json)
    );
}

#[rstest]
#[case::empty_list("[]")]
#[case::empty_text(r#""""#)]
#[case::zero("0")]
#[case::zero_float("0.0")]
#[case::boolean_false("false")]
#[case::empty_object("{}")]
fn stream_lookup_returns_falsey_values_that_are_not_null(#[case] value: &str) {
    let chunk = chunk("vertex_ai_grounding_metadata", value);
    assert_eq!(
        stream_chunk_attr(&chunk, "vertex_ai_grounding_metadata").cloned(),
        Some(json(value))
    );
}

#[test]
fn an_empty_top_level_list_shadows_populated_hidden_metadata() {
    let chunks = [json(
        r#"{"vertex_ai_grounding_metadata": [], "_hidden_params": {"vertex_ai_grounding_metadata": ["hidden"]}}"#,
    )];
    assert_eq!(
        stream_chunk_attr(&chunks[0], "vertex_ai_grounding_metadata").cloned(),
        Some(json("[]"))
    );
    assert!(
        assemble_stream_metadata(&chunks)
            .grounding_metadata
            .is_empty()
    );
}

#[test]
fn assembly_skips_falsey_chunk_values() {
    let chunks: Vec<Json> = ["[]", r#""""#, "0", "0.0", "false", "{}", "null"]
        .into_iter()
        .map(|value| chunk("vertex_ai_grounding_metadata", value))
        .collect();
    assert_eq!(
        assemble_stream_metadata(&chunks),
        MetadataExtension::default()
    );
}

#[test]
fn assembly_keeps_chunk_order_and_duplicate_entries() {
    let extension = assemble_stream_metadata(&[
        chunk("vertex_ai_grounding_metadata", r#"[{"n": 1}, {"n": 2}]"#),
        chunk("vertex_ai_grounding_metadata", r#"[{"n": 1}]"#),
        chunk("vertex_ai_grounding_metadata", r#"[{"n": 1}, {"n": 2}]"#),
    ]);
    assert_eq!(
        extension.grounding_metadata,
        vec![
            json(r#"{"n": 1}"#),
            json(r#"{"n": 2}"#),
            json(r#"{"n": 1}"#),
            json(r#"{"n": 1}"#),
            json(r#"{"n": 2}"#),
        ]
    );
}

#[test]
fn assembly_wraps_a_non_list_chunk_value() {
    let extension = assemble_stream_metadata(&[
        chunk("vertex_ai_citation_metadata", r#""one""#),
        chunk("vertex_ai_citation_metadata", r#"{"citations": []}"#),
    ]);
    assert_eq!(
        extension.citation_metadata,
        vec![json(r#""one""#), json(r#"{"citations": []}"#)]
    );
}

#[test]
fn assembly_reads_safety_ratings_and_safety_results_independently() {
    let extension = assemble_stream_metadata(&[json(
        r#"{"vertex_ai_safety_ratings": [["rating"]], "vertex_ai_safety_results": [["other"]]}"#,
    )]);
    assert_eq!(extension.safety_ratings, vec![json(r#"["rating"]"#)]);
    assert_eq!(extension.safety_results, vec![json(r#"["other"]"#)]);
}

#[test]
fn every_metadata_field_assembles_from_its_own_wire_name() {
    let extension = assemble_stream_metadata(&[json(
        r#"{
            "vertex_ai_url_context_metadata": [{"urlMetadata": []}],
            "vertex_ai_citation_metadata": [{"citations": []}],
            "vertex_ai_safety_results": [["result"]]
        }"#,
    )]);
    assert_eq!(
        extension.url_context_metadata,
        vec![json(r#"{"urlMetadata": []}"#)]
    );
    assert_eq!(
        extension.citation_metadata,
        vec![json(r#"{"citations": []}"#)]
    );
    assert_eq!(extension.safety_results, vec![json(r#"["result"]"#)]);
    assert!(extension.grounding_metadata.is_empty());
    assert!(extension.safety_ratings.is_empty());
}

#[test]
fn metadata_fields_follow_the_shared_wire_names() {
    assert_eq!(
        MetadataField::ALL.map(MetadataField::wire_name),
        VERTEX_METADATA_FIELDS
    );
}

#[test]
fn complete_metadata_lists_every_field_and_hidden_copies_skip_empty_ones() {
    let extension = extension_with(vec![json(r#"{"n": 1}"#)]);
    let fields = all_fields(&extension);
    assert_eq!(fields.len(), 5);
    assert_eq!(fields[0].0, "vertex_ai_grounding_metadata");
    assert_eq!(fields[0].1, extension.grounding_metadata.as_slice());
    assert_eq!(fields[4].0, "vertex_ai_citation_metadata");
    assert!(fields[4].1.is_empty());
    assert_eq!(
        extension.populated_fields(),
        vec![(
            "vertex_ai_grounding_metadata",
            extension.grounding_metadata.as_slice()
        )]
    );
    assert!(MetadataExtension::default().populated_fields().is_empty());
}

#[test]
fn candidate_and_assembled_metadata_agree() {
    let candidate = json(
        r#"{
            "groundingMetadata": [{"groundingChunks": []}],
            "safetyRatings": [{"category": "HARM_CATEGORY_HATE_SPEECH"}],
            "citationMetadata": {"citations": []},
            "urlContextMetadata": {"urlMetadata": []}
        }"#,
    );
    let chunks = [json(
        r#"{
            "vertex_ai_grounding_metadata": [{"groundingChunks": []}],
            "vertex_ai_url_context_metadata": [{"urlMetadata": []}],
            "vertex_ai_safety_ratings": [[{"category": "HARM_CATEGORY_HATE_SPEECH"}]],
            "vertex_ai_safety_results": [[{"category": "HARM_CATEGORY_HATE_SPEECH"}]],
            "vertex_ai_citation_metadata": [{"citations": []}]
        }"#,
    )];
    let extracted = extract_candidate_metadata(&candidate);
    let assembled = assemble_stream_metadata(&chunks);
    assert_eq!(all_fields(&extracted), all_fields(&assembled));
    assert_eq!(extracted.populated_fields(), assembled.populated_fields());
}

fn web_metadata(supports: &str) -> Json {
    json(&format!(
        r#"{{"groundingChunks": [{{"retrievedContext": {{"uri": "gs://first"}}}}, {{"web": {{"uri": "https://second", "title": "Second"}}}}], "groundingSupports": [{supports}]}}"#
    ))
}

#[test]
fn annotations_use_only_the_first_grounding_chunk_index() {
    let metadata = web_metadata(
        r#"{"segment": {"startIndex": 0, "endIndex": 4}, "groundingChunkIndices": [0, 1]}"#,
    );
    assert!(grounding_annotations(&[metadata]).is_empty());
    let pointed_at_the_web_chunk = web_metadata(
        r#"{"segment": {"startIndex": 0, "endIndex": 4}, "groundingChunkIndices": [1, 0]}"#,
    );
    assert_eq!(
        grounding_annotations(&[pointed_at_the_web_chunk]),
        vec![GroundingAnnotation {
            kind: AnnotationType::UrlCitation,
            url_citation: UrlCitation {
                start_index: 0,
                end_index: 4,
                url: "https://second".to_owned(),
                title: "Second".to_owned(),
            },
        }]
    );
}

#[rstest]
#[case::zero_bounds("0", "0")]
#[case::zero_length("3", "3")]
fn annotations_accept_zero_valued_segment_indices(#[case] start: &str, #[case] end: &str) {
    let metadata = web_metadata(&format!(
        r#"{{"segment": {{"startIndex": {start}, "endIndex": {end}}}, "groundingChunkIndices": [1]}}"#
    ));
    let annotations = grounding_annotations(&[metadata]);
    assert_eq!(annotations.len(), 1);
    assert_eq!(
        annotations[0].url_citation.start_index,
        start.parse::<i64>().unwrap()
    );
    assert_eq!(
        annotations[0].url_citation.end_index,
        end.parse::<i64>().unwrap()
    );
}

#[rstest]
#[case::missing_start(r#"{"segment": {"endIndex": 4}, "groundingChunkIndices": [1]}"#)]
#[case::missing_end(r#"{"segment": {"startIndex": 0}, "groundingChunkIndices": [1]}"#)]
#[case::null_start(
    r#"{"segment": {"startIndex": null, "endIndex": 4}, "groundingChunkIndices": [1]}"#
)]
#[case::missing_segment(r#"{"groundingChunkIndices": [1]}"#)]
#[case::no_chunk_indices(r#"{"segment": {"startIndex": 0, "endIndex": 4}}"#)]
#[case::empty_chunk_indices(
    r#"{"segment": {"startIndex": 0, "endIndex": 4}, "groundingChunkIndices": []}"#
)]
#[case::unknown_chunk_index(
    r#"{"segment": {"startIndex": 0, "endIndex": 4}, "groundingChunkIndices": [9]}"#
)]
fn annotations_need_both_segment_indices_and_a_web_chunk(#[case] support: &str) {
    assert!(grounding_annotations(&[web_metadata(support)]).is_empty());
}

#[test]
fn missing_uri_and_title_become_empty_strings() {
    let metadata = json(
        r#"{"groundingChunks": [{"web": {}}], "groundingSupports": [{"segment": {"startIndex": 2, "endIndex": 5}, "groundingChunkIndices": [0]}]}"#,
    );
    assert_eq!(
        grounding_annotations(&[metadata]),
        vec![GroundingAnnotation {
            kind: AnnotationType::UrlCitation,
            url_citation: UrlCitation {
                start_index: 2,
                end_index: 5,
                url: String::new(),
                title: String::new(),
            },
        }]
    );
}

#[test]
fn null_uri_and_title_take_the_empty_default() {
    let metadata = json(
        r#"{"groundingChunks": [{"web": {"uri": null, "title": null}}], "groundingSupports": [{"segment": {"startIndex": 0, "endIndex": 1}, "groundingChunkIndices": [0]}]}"#,
    );
    let annotations = grounding_annotations(&[metadata]);
    assert_eq!(annotations.len(), 1);
    assert_eq!(annotations[0].url_citation.url, "");
    assert_eq!(annotations[0].url_citation.title, "");
}

#[test]
fn annotations_keep_support_order_and_duplicates() {
    let metadata = json(
        r#"{"groundingChunks": [{"web": {"uri": "https://a", "title": "A"}}], "groundingSupports": [
            {"segment": {"startIndex": 0, "endIndex": 1}, "groundingChunkIndices": [0]},
            {"segment": {"startIndex": 0, "endIndex": 1}, "groundingChunkIndices": [0]},
            {"segment": {"startIndex": 2, "endIndex": 3}, "groundingChunkIndices": [0]}
        ]}"#,
    );
    let annotations = grounding_annotations(&[metadata]);
    assert_eq!(
        annotations
            .iter()
            .map(|annotation| (
                annotation.url_citation.start_index,
                annotation.url_citation.end_index
            ))
            .collect::<Vec<_>>(),
        vec![(0, 1), (0, 1), (2, 3)]
    );
}

#[test]
fn annotations_walk_every_grounding_metadata_entry() {
    let first = json(
        r#"{"groundingChunks": [{"web": {"uri": "https://a"}}], "groundingSupports": [{"segment": {"startIndex": 0, "endIndex": 1}, "groundingChunkIndices": [0]}]}"#,
    );
    let second = json(
        r#"{"groundingChunks": [{"web": {"uri": "https://b"}}], "groundingSupports": [{"segment": {"startIndex": 2, "endIndex": 3}, "groundingChunkIndices": [0]}]}"#,
    );
    let annotations = grounding_annotations(&[first, second]);
    assert_eq!(annotations.len(), 2);
    assert_eq!(annotations[0].url_citation.url, "https://a");
    assert_eq!(annotations[1].url_citation.url, "https://b");
}

#[test]
fn annotations_skip_non_object_metadata_and_wrong_typed_positions() {
    let metadata = json(
        r#"{"groundingChunks": "chunk", "groundingSupports": [{"segment": "text", "groundingChunkIndices": [0]}, 7]}"#,
    );
    assert!(grounding_annotations(&[json("1"), metadata]).is_empty());
}
