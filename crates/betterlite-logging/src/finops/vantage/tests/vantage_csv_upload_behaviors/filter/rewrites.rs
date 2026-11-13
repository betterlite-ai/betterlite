use rstest::rstest;

use crate::finops::vantage::columns::strip_unsupported_columns;

#[rstest]
#[case::all_supported_keeps_line_feeds(
    "ChargeCategory,BilledCost\nA,1\n",
    "ChargeCategory,BilledCost\nA,1\n"
)]
#[case::drops_unsupported_and_writes_crlf(
    "ChargeCategory,ProviderName,BilledCost\nUsage,openai,1.5\n",
    "ChargeCategory,BilledCost\r\nUsage,1.5\r\n"
)]
#[case::quoted_comma_stays_one_field(
    "ChargeCategory,Junk,ServiceName\nA,1,\"Compute, large\"\n",
    "ChargeCategory,ServiceName\r\nA,\"Compute, large\"\r\n"
)]
#[case::quoted_header_names_match(
    "\"ChargeCategory\",\"Junk\"\n\"A\",\"B\"\n",
    "ChargeCategory\r\nA\r\n"
)]
#[case::blank_lines_are_skipped(
    "ChargeCategory,Junk\nA,1\n\nB,2\n",
    "ChargeCategory\r\nA\r\nB\r\n"
)]
#[case::newline_inside_a_cell_stays_one_record(
    "ChargeCategory,Junk\n\"line1\nline2\",1\n",
    "ChargeCategory\r\n\"line1\nline2\"\r\n"
)]
#[case::quotes_inside_a_cell_are_doubled(
    "ChargeCategory,Junk\n\"say \"\"hi\"\"\",1\n",
    "ChargeCategory\r\n\"say \"\"hi\"\"\"\r\n"
)]
#[case::lone_empty_field_is_quoted("ChargeCategory,Junk\n,1\n", "ChargeCategory\r\n\"\"\r\n")]
#[case::empty_fields_among_several_stay_bare(
    "ChargeCategory,BilledCost,Junk\n,,1\n",
    "ChargeCategory,BilledCost\r\n,\r\n"
)]
#[case::surrounding_spaces_stay_unquoted(
    "ChargeCategory,Junk\n a ,1\n",
    "ChargeCategory\r\n a \r\n"
)]
#[case::unicode_survives(
    "ChargeCategory,Junk\n\u{c9}t\u{e9},1\n",
    "ChargeCategory\r\n\u{c9}t\u{e9}\r\n"
)]
#[case::unsupported_header_with_space_is_dropped(
    " ChargeCategory,ServiceName\nA,B\n",
    "ServiceName\r\nB\r\n"
)]
#[case::every_column_dropped_leaves_empty_records("Junk,Other\n1,2\n", "\r\n\r\n")]
#[case::crlf_input_keeps_its_last_supported_column(
    "Junk,ServiceName\r\n1,Compute\r\n",
    "ServiceName\r\nCompute\r\n"
)]
#[case::crlf_input_with_all_columns_supported_is_unchanged(
    "ChargeCategory,ServiceName\r\nA,B\r\n",
    "ChargeCategory,ServiceName\r\nA,B\r\n"
)]
#[case::body_is_not_decoded_when_nothing_is_dropped(
    "ChargeCategory,ServiceName\nA,B\n",
    "ChargeCategory,ServiceName\nA,B\n"
)]
fn filtered_content_matches(#[case] input: &str, #[case] expected: &str) {
    let filtered = strip_unsupported_columns(input.as_bytes()).unwrap();
    assert_eq!(String::from_utf8(filtered.into_owned()).unwrap(), expected);
}

#[test]
fn invalid_utf8_after_the_header_passes_when_nothing_is_dropped() {
    let input = b"ChargeCategory,ServiceName\n\xff,B\n";
    let filtered = strip_unsupported_columns(input).unwrap();
    assert_eq!(filtered.as_ref(), input);
}
