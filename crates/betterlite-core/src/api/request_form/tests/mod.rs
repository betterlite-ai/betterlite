#![cfg(test)]

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rstest::rstest;
use serde_json::Number;

use crate::api::request_form::annotations::{
    FormAnnotation, NumericType, Qualifier, numeric_form_fields, numeric_form_type,
};
use crate::api::request_form::coercion::{coerce_numeric_form_fields, float_value};
use crate::api::request_form::convert::{
    ConvertError, FileDataValue, FileTuple, convert_uploads_to_file_data,
};
use crate::api::request_form::metadata::{
    MetadataTree, MetadataValue, MetadataWarning, MetadataWarningKind, extract_nested_form_metadata,
};
use crate::api::request_form::scalars::{scalar_float, scalar_int};
use crate::api::request_form::values::{
    FormError, FormFields, FormPart, FormParts, FormValue, NonFinite, PartValue, UploadFile,
    get_form_data,
};

fn text(name: &str, value: &str) -> FormPart {
    FormPart {
        name: name.to_owned(),
        value: PartValue::Text(value.to_owned()),
    }
}

fn upload(name: &str, filename: &str) -> FormPart {
    FormPart {
        name: name.to_owned(),
        value: PartValue::Upload(UploadFile {
            filename: filename.to_owned(),
            content_type: Some("text/plain".to_owned()),
            content: b"abc".to_vec(),
            size: Some(3),
        }),
    }
}

fn parts(items: Vec<FormPart>) -> FormParts {
    FormParts { items }
}

fn fields(entries: Vec<(&str, FormValue)>) -> FormFields {
    entries
        .into_iter()
        .map(|(name, value)| (name.to_owned(), value))
        .collect()
}

fn texts(entries: &[(&str, &str)]) -> FormFields {
    entries
        .iter()
        .map(|(name, value)| ((*name).to_owned(), FormValue::Text((*value).to_owned())))
        .collect()
}

fn json_text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

#[test]
fn repeated_plain_last_wins() {
    let decoded =
        get_form_data(&parts(vec![text("model", "first"), text("model", "last")])).unwrap();
    assert_eq!(
        decoded.get("model"),
        Some(&FormValue::Text("last".to_owned()))
    );
}

#[test]
fn repeated_plain_keeps_the_first_position() {
    let decoded = get_form_data(&parts(vec![
        text("a", "1"),
        text("model", "first"),
        text("model", "last"),
    ]))
    .unwrap();
    assert_eq!(
        decoded.keys().map(String::as_str).collect::<Vec<_>>(),
        vec!["a", "model"]
    );
}

#[test]
fn array_entries_append_in_order() {
    let decoded = get_form_data(&parts(vec![
        text("timestamp_granularities[]", "word"),
        text("timestamp_granularities[]", "segment"),
    ]))
    .unwrap();
    assert_eq!(
        decoded.get("timestamp_granularities"),
        Some(&FormValue::List(vec![
            FormValue::Text("word".to_owned()),
            FormValue::Text("segment".to_owned()),
        ]))
    );
}

#[test]
fn a_plain_value_after_an_array_overwrites_the_list() {
    let decoded = get_form_data(&parts(vec![text("x[]", "child"), text("x", "leaf")])).unwrap();
    assert_eq!(decoded.get("x"), Some(&FormValue::Text("leaf".to_owned())));
}

#[test]
fn an_array_entry_after_a_scalar_fails() {
    let error = get_form_data(&parts(vec![text("x", "leaf"), text("x[]", "child")])).unwrap_err();
    assert_eq!(
        error,
        FormError::ScalarAppend {
            name: "x".to_owned()
        }
    );
}

#[test]
fn an_upload_can_join_an_array() {
    let decoded = get_form_data(&parts(vec![upload("files[]", "a.txt")])).unwrap();
    assert!(matches!(
        decoded.get("files"),
        Some(FormValue::List(items)) if matches!(items.first(), Some(FormValue::Upload(_)))
    ));
}

#[test]
fn an_empty_form_has_no_fields() {
    assert!(get_form_data(&parts(Vec::new())).unwrap().is_empty());
}

#[rstest]
#[case(FormAnnotation::Int, Some(NumericType::Int))]
#[case(FormAnnotation::Float, Some(NumericType::Float))]
#[case(FormAnnotation::Other, None)]
fn a_bare_scalar_is_numeric_only_for_int_and_float(
    #[case] annotation: FormAnnotation,
    #[case] expected: Option<NumericType>,
) {
    assert_eq!(numeric_form_type(&annotation), expected);
}

#[rstest]
#[case(FormAnnotation::Union(vec![FormAnnotation::Int, FormAnnotation::NoneType]), Some(NumericType::Int))]
#[case(FormAnnotation::Union(vec![FormAnnotation::Float, FormAnnotation::NoneType]), Some(NumericType::Float))]
#[case(FormAnnotation::Union(vec![FormAnnotation::Int, FormAnnotation::Float]), None)]
#[case(FormAnnotation::Union(vec![FormAnnotation::NoneType]), None)]
#[case(FormAnnotation::Union(Vec::new()), None)]
fn only_a_single_non_none_union_member_is_numeric(
    #[case] annotation: FormAnnotation,
    #[case] expected: Option<NumericType>,
) {
    assert_eq!(numeric_form_type(&annotation), expected);
}

#[rstest]
#[case(Qualifier::Annotated)]
#[case(Qualifier::NotRequired)]
#[case(Qualifier::ReadOnly)]
#[case(Qualifier::Required)]
fn a_qualifier_around_a_scalar_is_peeled(#[case] qualifier: Qualifier) {
    let annotation = FormAnnotation::Qualified(qualifier, Box::new(FormAnnotation::Int));
    assert_eq!(numeric_form_type(&annotation), Some(NumericType::Int));
}

#[test]
fn nested_qualifiers_are_peeled() {
    let annotation = FormAnnotation::Qualified(
        Qualifier::Required,
        Box::new(FormAnnotation::Qualified(
            Qualifier::NotRequired,
            Box::new(FormAnnotation::Float),
        )),
    );
    assert_eq!(numeric_form_type(&annotation), Some(NumericType::Float));
    let qualified_union = FormAnnotation::Qualified(
        Qualifier::Annotated,
        Box::new(FormAnnotation::Union(vec![
            FormAnnotation::Int,
            FormAnnotation::NoneType,
        ])),
    );
    assert_eq!(numeric_form_type(&qualified_union), Some(NumericType::Int));
}

#[test]
fn a_qualifier_inside_a_union_is_not_peeled() {
    let annotation = FormAnnotation::Union(vec![
        FormAnnotation::Qualified(Qualifier::Annotated, Box::new(FormAnnotation::Int)),
        FormAnnotation::NoneType,
    ]);
    assert_eq!(numeric_form_type(&annotation), None);
}

#[test]
fn numeric_form_fields_keeps_only_the_eligible_names() {
    let annotations: IndexMap<String, FormAnnotation> = [
        ("count".to_owned(), FormAnnotation::Int),
        ("ratio".to_owned(), FormAnnotation::Float),
        ("flag".to_owned(), FormAnnotation::Other),
        (
            "optional".to_owned(),
            FormAnnotation::Union(vec![FormAnnotation::Float, FormAnnotation::NoneType]),
        ),
    ]
    .into_iter()
    .collect();
    let numeric = numeric_form_fields(&annotations);
    assert_eq!(
        numeric.keys().map(String::as_str).collect::<Vec<_>>(),
        vec!["count", "ratio", "optional"]
    );
    assert_eq!(numeric.get("optional"), Some(&NumericType::Float));
}

#[rstest]
#[case(" 7 ", "7")]
#[case("+3", "3")]
#[case("1_000", "1000")]
#[case("١٢", "12")]
#[case("007", "7")]
#[case("-0", "0")]
#[case("+0", "0")]
#[case("-5", "-5")]
#[case("1٢", "12")]
#[case("0_0", "0")]
#[case(" 5", "5")]
#[case("\u{1c}5", "5")]
#[case("123456789012345678901234567890", "123456789012345678901234567890")]
fn an_integer_text_parses_like_python(#[case] source: &str, #[case] expected: &str) {
    assert_eq!(
        scalar_int(source).map(|value| value.to_string()),
        Some(expected.to_owned())
    );
}

#[rstest]
#[case("+")]
#[case("-")]
#[case("3.5")]
#[case("nan")]
#[case("inf")]
#[case("bad")]
#[case("")]
#[case("1__0")]
#[case("_1")]
#[case("1_")]
#[case("0x10")]
#[case("1,5")]
#[case("1 2")]
#[case("0 1")]
#[case("Ⅻ")]
fn a_non_integer_text_stays_unparsed(#[case] source: &str) {
    assert_eq!(scalar_int(source), None);
}

#[rstest]
#[case(" 7 ", 7.0)]
#[case("+3", 3.0)]
#[case("1_000", 1000.0)]
#[case("1e2", 100.0)]
#[case(".5", 0.5)]
#[case("5.", 5.0)]
#[case("1.e5", 100_000.0)]
#[case("1_0.5_5e1_0", 105_500_000_000.0)]
#[case("١٢", 12.0)]
#[case("1e-400", 0.0)]
#[case("-0", -0.0)]
fn a_float_text_parses_like_python(#[case] source: &str, #[case] expected: f64) {
    let parsed = scalar_float(source).unwrap();
    assert_eq!(parsed.to_bits(), expected.to_bits());
}

#[test]
fn a_float_text_accepts_python_infinities_and_nan() {
    assert!(scalar_float("inf").unwrap().is_infinite());
    assert!(scalar_float("inf").unwrap().is_sign_positive());
    assert!(scalar_float("-Infinity").unwrap().is_sign_negative());
    assert!(scalar_float("1e400").unwrap().is_infinite());
    assert!(scalar_float("nan").unwrap().is_nan());
    assert!(scalar_float("NaN").unwrap().is_nan());
}

#[rstest]
#[case("bad")]
#[case("")]
#[case("1__0")]
#[case("1_.5")]
#[case("1._5")]
#[case("1_e5")]
#[case("0x10")]
#[case("1,5")]
fn a_non_float_text_stays_unparsed(#[case] source: &str) {
    assert_eq!(scalar_float(source), None);
}

#[test]
fn numeric_coercion_parses_only_the_listed_fields() {
    let numeric = numeric_form_fields(
        &[
            ("count".to_owned(), FormAnnotation::Int),
            ("ratio".to_owned(), FormAnnotation::Float),
        ]
        .into_iter()
        .collect(),
    );
    let source = texts(&[("count", " 7 "), ("ratio", "1e2"), ("other", "9")]);
    let coerced = coerce_numeric_form_fields(&source, &numeric);
    assert_eq!(
        coerced.get("count"),
        Some(&FormValue::Number(Number::from(7)))
    );
    assert_eq!(
        coerced.get("ratio"),
        Some(&FormValue::Number(Number::from_f64(100.0).unwrap()))
    );
    assert_eq!(coerced.get("other"), Some(&FormValue::Text("9".to_owned())));
    assert_eq!(
        coerced.keys().map(String::as_str).collect::<Vec<_>>(),
        source.keys().map(String::as_str).collect::<Vec<_>>()
    );
}

#[test]
fn an_unparsable_text_for_a_numeric_field_stays_a_string() {
    let numeric = numeric_form_fields(
        &[
            ("count".to_owned(), FormAnnotation::Int),
            ("ratio".to_owned(), FormAnnotation::Float),
        ]
        .into_iter()
        .collect(),
    );
    let coerced =
        coerce_numeric_form_fields(&texts(&[("count", "3.5"), ("ratio", "bad")]), &numeric);
    assert_eq!(
        coerced.get("count"),
        Some(&FormValue::Text("3.5".to_owned()))
    );
    assert_eq!(
        coerced.get("ratio"),
        Some(&FormValue::Text("bad".to_owned()))
    );
}

#[test]
fn an_int_field_keeps_nan_and_inf_as_strings() {
    let numeric = numeric_form_fields(
        &[("count".to_owned(), FormAnnotation::Int)]
            .into_iter()
            .collect(),
    );
    let coerced = coerce_numeric_form_fields(&texts(&[("count", "nan")]), &numeric);
    assert_eq!(
        coerced.get("count"),
        Some(&FormValue::Text("nan".to_owned()))
    );
}

#[test]
fn non_string_values_are_left_alone() {
    let numeric = numeric_form_fields(
        &[("count".to_owned(), FormAnnotation::Int)]
            .into_iter()
            .collect(),
    );
    let source = fields(vec![("count", FormValue::Other(Json::Bool(true)))]);
    let coerced = coerce_numeric_form_fields(&source, &numeric);
    assert_eq!(
        coerced.get("count"),
        Some(&FormValue::Other(Json::Bool(true)))
    );
}

#[test]
fn a_float_text_carries_non_finite_values_as_variants() {
    let numeric = numeric_form_fields(
        &[("ratio".to_owned(), FormAnnotation::Float)]
            .into_iter()
            .collect(),
    );
    let coerced = coerce_numeric_form_fields(&texts(&[("ratio", "nan")]), &numeric);
    assert_eq!(
        coerced.get("ratio"),
        Some(&FormValue::NonFinite(NonFinite::Nan))
    );
    let positive = coerce_numeric_form_fields(&texts(&[("ratio", "inf")]), &numeric);
    assert_eq!(
        positive.get("ratio"),
        Some(&FormValue::NonFinite(NonFinite::PositiveInfinity))
    );
    let negative = coerce_numeric_form_fields(&texts(&[("ratio", "-Infinity")]), &numeric);
    assert_eq!(
        negative.get("ratio"),
        Some(&FormValue::NonFinite(NonFinite::NegativeInfinity))
    );
}

#[test]
fn every_non_finite_value_has_its_wire_token() {
    assert_eq!(float_value(f64::NAN), FormValue::NonFinite(NonFinite::Nan));
    assert_eq!(
        float_value(f64::INFINITY),
        FormValue::NonFinite(NonFinite::PositiveInfinity)
    );
    assert_eq!(
        float_value(f64::NEG_INFINITY),
        FormValue::NonFinite(NonFinite::NegativeInfinity)
    );
    assert_eq!(NonFinite::Nan.token(), "nan");
    assert_eq!(NonFinite::PositiveInfinity.token(), "positive_infinity");
    assert_eq!(NonFinite::NegativeInfinity.token(), "negative_infinity");
}

fn uploaded(filename: &str, content: &str) -> FormValue {
    FormValue::Upload(UploadFile {
        filename: filename.to_owned(),
        content_type: Some("text/plain".to_owned()),
        content: content.as_bytes().to_vec(),
        size: Some(u64::try_from(content.len()).unwrap()),
    })
}

#[test]
fn a_single_upload_becomes_a_one_element_list() {
    let data =
        convert_uploads_to_file_data(&fields(vec![("file", uploaded("a.txt", "abc"))])).unwrap();
    assert_eq!(
        data.get("file"),
        Some(&FileDataValue::Files(vec![FileTuple {
            filename: "a.txt".to_owned(),
            content: b"abc".to_vec(),
            content_type: Some("text/plain".to_owned()),
        }]))
    );
}

#[test]
fn an_upload_list_is_read_in_order() {
    let data = convert_uploads_to_file_data(&fields(vec![(
        "files",
        FormValue::List(vec![uploaded("a.txt", "a"), uploaded("b.txt", "b")]),
    )]))
    .unwrap();
    let Some(FileDataValue::Files(files)) = data.get("files") else {
        panic!("the list should convert");
    };
    assert_eq!(
        files
            .iter()
            .map(|file| file.filename.as_str())
            .collect::<Vec<_>>(),
        vec!["a.txt", "b.txt"]
    );
}

#[test]
fn a_list_starting_with_text_is_unchanged() {
    let source = FormValue::List(vec![
        FormValue::Text("plain".to_owned()),
        uploaded("a.txt", "a"),
    ]);
    let data = convert_uploads_to_file_data(&fields(vec![("files", source.clone())])).unwrap();
    assert_eq!(data.get("files"), Some(&FileDataValue::Unchanged(source)));
}

#[test]
fn a_list_starting_with_an_upload_fails_at_the_text_entry() {
    let error = convert_uploads_to_file_data(&fields(vec![(
        "files",
        FormValue::List(vec![
            uploaded("a.txt", "a"),
            FormValue::Text("plain".to_owned()),
        ]),
    )]))
    .unwrap_err();
    assert_eq!(
        error,
        ConvertError::MissingRead {
            name: "files".to_owned(),
            index: 1,
        }
    );
}

#[test]
fn an_empty_list_stays_empty() {
    let data = convert_uploads_to_file_data(&fields(vec![("files", FormValue::List(Vec::new()))]))
        .unwrap();
    assert_eq!(
        data.get("files"),
        Some(&FileDataValue::Unchanged(FormValue::List(Vec::new())))
    );
}

#[test]
fn plain_fields_are_unchanged() {
    let source = FormValue::Text("1".to_owned());
    let data = convert_uploads_to_file_data(&fields(vec![("x", source.clone())])).unwrap();
    assert_eq!(data.get("x"), Some(&FileDataValue::Unchanged(source)));
}

fn leaf(value: &str) -> MetadataValue {
    MetadataValue::Leaf(FormValue::Text(value.to_owned()))
}

fn object<const N: usize>(entries: [(&str, MetadataValue); N]) -> MetadataValue {
    MetadataValue::Object(
        entries
            .into_iter()
            .map(|(name, value)| (name.to_owned(), value))
            .collect(),
    )
}

fn metadata_of(entries: &[(&str, &str)]) -> MetadataTree {
    extract_nested_form_metadata(&texts(entries), "litellm_metadata[").0
}

fn warnings_of(entries: &[(&str, &str)]) -> Vec<MetadataWarning> {
    extract_nested_form_metadata(&texts(entries), "litellm_metadata[").1
}

#[test]
fn a_bracketed_path_nests() {
    assert_eq!(
        metadata_of(&[("litellm_metadata[a][b]", "v")]),
        object([("a", object([("b", leaf("v"))]))])
    );
}

#[test]
fn a_scalar_parent_keeps_its_value_and_warns() {
    assert_eq!(
        metadata_of(&[
            ("litellm_metadata[a]", "leaf"),
            ("litellm_metadata[a][b]", "deep")
        ]),
        object([("a", leaf("leaf"))])
    );
    assert_eq!(
        warnings_of(&[
            ("litellm_metadata[a]", "leaf"),
            ("litellm_metadata[a][b]", "deep")
        ]),
        vec![MetadataWarning {
            kind: MetadataWarningKind::ParentNotObject,
            subject: "litellm_metadata[a][b]".to_owned(),
        }]
    );
}

#[test]
fn a_later_scalar_leaf_overwrites_the_subtree_without_a_warning() {
    assert_eq!(
        metadata_of(&[
            ("litellm_metadata[a][b]", "deep"),
            ("litellm_metadata[a]", "leaf")
        ]),
        object([("a", leaf("leaf"))])
    );
    assert!(
        warnings_of(&[
            ("litellm_metadata[a][b]", "deep"),
            ("litellm_metadata[a]", "leaf")
        ])
        .is_empty()
    );
}

#[test]
fn a_deep_scalar_parent_warns_at_the_next_part() {
    assert_eq!(
        metadata_of(&[
            ("litellm_metadata[a]", "leaf"),
            ("litellm_metadata[a][b][c]", "deep")
        ]),
        object([("a", leaf("leaf"))])
    );
    assert_eq!(
        warnings_of(&[
            ("litellm_metadata[a]", "leaf"),
            ("litellm_metadata[a][b][c]", "deep")
        ]),
        vec![MetadataWarning {
            kind: MetadataWarningKind::IntermediateNotObject,
            subject: "b".to_owned(),
        }]
    );
}

#[test]
fn trailing_brackets_are_stripped() {
    assert_eq!(
        metadata_of(&[("litellm_metadata[a]]]", "v")]),
        object([("a", leaf("v"))])
    );
}

#[test]
fn every_prefix_occurrence_is_removed() {
    assert_eq!(
        metadata_of(&[("litellm_metadata[litellm_metadata[a]]", "v")]),
        object([("a", leaf("v"))])
    );
}

#[test]
fn a_trailing_empty_part_keys_the_empty_string() {
    assert_eq!(
        metadata_of(&[("litellm_metadata[a][]", "v")]),
        object([("a", object([("", leaf("v"))]))])
    );
}

#[test]
fn an_intermediate_empty_part_is_preserved() {
    assert_eq!(
        metadata_of(&[("litellm_metadata[a][][b]", "v")]),
        object([("a", object([("", object([("b", leaf("v"))]))]))])
    );
}

#[rstest]
#[case(128, false)]
#[case(129, true)]
#[case(50_000, true)]
fn metadata_depth_is_bounded(#[case] depth: usize, #[case] rejected: bool) {
    let path = vec!["a"; depth].join("][");
    let key = format!("litellm_metadata[{path}]");
    let (metadata, warnings) =
        extract_nested_form_metadata(&texts(&[(&key, "v")]), "litellm_metadata[");
    if rejected {
        assert!(metadata.is_empty());
        assert_eq!(
            warnings,
            vec![MetadataWarning {
                kind: MetadataWarningKind::PathTooDeep,
                subject: key,
            }]
        );
    } else {
        assert!(warnings.is_empty());
        let mut cursor = &metadata;
        for _level in 1..depth {
            let MetadataValue::Object(child) = cursor.get("a").unwrap() else {
                panic!("expected another object");
            };
            cursor = child;
        }
        assert_eq!(cursor.get("a"), Some(&leaf("v")));
    }
}

#[test]
fn an_empty_path_warns_and_drops() {
    assert!(metadata_of(&[("litellm_metadata[]", "v")]).is_empty());
    assert_eq!(
        warnings_of(&[("litellm_metadata[]", "v")]),
        vec![MetadataWarning {
            kind: MetadataWarningKind::EmptyPath,
            subject: "litellm_metadata[]".to_owned(),
        }]
    );
}

#[test]
fn an_upload_value_is_skipped_with_a_warning() {
    let source = fields(vec![("litellm_metadata[a]", uploaded("a.txt", "abc"))]);
    let (metadata, warnings) = extract_nested_form_metadata(&source, "litellm_metadata[");
    assert!(metadata.is_empty());
    assert_eq!(
        warnings,
        vec![MetadataWarning {
            kind: MetadataWarningKind::UploadSkipped,
            subject: "litellm_metadata[a]".to_owned(),
        }]
    );
}

#[test]
fn other_keys_are_ignored() {
    assert!(metadata_of(&[("team", "t"), ("litellm_metadata_other[a]", "v")]).is_empty());
}

#[test]
fn an_explicit_prefix_is_used() {
    let (metadata, warnings) =
        extract_nested_form_metadata(&texts(&[("metadata[team]", "t")]), "metadata[");
    assert_eq!(metadata, object([("team", leaf("t"))]));
    assert!(warnings.is_empty());
}

#[test]
fn an_empty_form_yields_no_metadata() {
    let (metadata, warnings) =
        extract_nested_form_metadata(&FormFields::new(), "litellm_metadata[");
    assert!(metadata.is_empty());
    assert!(warnings.is_empty());
}

#[test]
fn an_intermediate_list_is_a_leaf() {
    let source = fields(vec![(
        "litellm_metadata[a]",
        FormValue::List(vec![FormValue::Text("x".to_owned())]),
    )]);
    let (metadata, _warnings) = extract_nested_form_metadata(&source, "litellm_metadata[");
    assert_eq!(
        metadata,
        object([(
            "a",
            MetadataValue::Leaf(FormValue::List(vec![FormValue::Text("x".to_owned())]))
        )])
    );
}

mod file_size;
