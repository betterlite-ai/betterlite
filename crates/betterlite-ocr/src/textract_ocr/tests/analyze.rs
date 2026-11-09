use std::collections::HashMap;
use std::iter::repeat_n;

use rstest::rstest;

use crate::ocr_config::OcrProviderConfig as _;
use crate::ocr_error::OcrError;
use crate::request_format::RequestFormat;
use crate::response::OCRResponse;
use crate::textract_ocr::analyze::TextractAnalyzeDocumentConfig;
use crate::textract_ocr::markdown::{WorkBudget, layout_markdown};
use crate::textract_ocr::wire::{Block, TextractResponse};

const MEBIBYTE: usize = 0x0010_0000;

const MODEL: &str = "analyze-document";

fn analyze(blocks: &str) -> Result<OCRResponse, OcrError> {
    let raw = format!(r#"{{"DocumentMetadata":{{"Pages":1}},"Blocks":{blocks}}}"#);
    TextractAnalyzeDocumentConfig.transform_ocr_response(
        MODEL,
        raw.as_bytes(),
        RequestFormat::Litellm,
    )
}

fn pages(response: &OCRResponse) -> Vec<(i64, &str)> {
    response
        .pages
        .iter()
        .map(|page| (page.index, page.markdown.as_str()))
        .collect()
}

#[rstest]
#[case::headings_paragraphs_and_a_list_without_repeating_its_items(
    r#"[
        {"Id":"l1","BlockType":"LINE","Text":"Quarterly Report"},
        {"Id":"l2","BlockType":"LINE","Text":"This report lists"},
        {"Id":"l3","BlockType":"LINE","Text":"the invoices."},
        {"Id":"l4","BlockType":"LINE","Text":"Line items"},
        {"Id":"l5","BlockType":"LINE","Text":"- Pay within 30 days"},
        {"Id":"l6","BlockType":"LINE","Text":"\u2022 Quote the number"},
        {"Id":"t","BlockType":"LAYOUT_TITLE","Relationships":[{"Type":"CHILD","Ids":["l1"]}]},
        {"Id":"p","BlockType":"LAYOUT_TEXT","Relationships":[{"Type":"CHILD","Ids":["l2","l3"]}]},
        {"Id":"h","BlockType":"LAYOUT_SECTION_HEADER","Relationships":[{"Type":"CHILD","Ids":["l4"]}]},
        {"Id":"ul","BlockType":"LAYOUT_LIST","Relationships":[{"Type":"CHILD","Ids":["i1","i2"]}]},
        {"Id":"i1","BlockType":"LAYOUT_TEXT","Relationships":[{"Type":"CHILD","Ids":["l5"]}]},
        {"Id":"i2","BlockType":"LAYOUT_TEXT","Relationships":[{"Type":"CHILD","Ids":["l6"]}]}]"#,
    vec![(0, "# Quarterly Report\n\nThis report lists the invoices.\n\n## Line items\n\n- Pay within 30 days\n- Quote the number")]
)]
#[case::header_footer_and_page_number_stay_in_reading_order(
    r#"[
        {"Id":"l1","BlockType":"LINE","Text":"ACME Corp"},
        {"Id":"l2","BlockType":"LINE","Text":"Body"},
        {"Id":"l3","BlockType":"LINE","Text":"Confidential"},
        {"Id":"l4","BlockType":"LINE","Text":"3"},
        {"Id":"hd","BlockType":"LAYOUT_HEADER","Relationships":[{"Type":"CHILD","Ids":["l1"]}]},
        {"Id":"p","BlockType":"LAYOUT_TEXT","Relationships":[{"Type":"CHILD","Ids":["l2"]}]},
        {"Id":"ft","BlockType":"LAYOUT_FOOTER","Relationships":[{"Type":"CHILD","Ids":["l3"]}]},
        {"Id":"number","BlockType":"LAYOUT_PAGE_NUMBER","Relationships":[{"Type":"CHILD","Ids":["l4"]}]}]"#,
    vec![(0, "ACME Corp\n\nBody\n\nConfidential\n\n3")]
)]
#[case::a_table_is_rendered_from_its_cells_in_row_and_column_order(
    r#"[
        {"Id":"l1","BlockType":"LINE","Text":"Invoice"},
        {"Id":"l2","BlockType":"LINE","Text":"Total"},
        {"Id":"l3","BlockType":"LINE","Text":"12345"},
        {"Id":"l4","BlockType":"LINE","Text":"a|b"},
        {"Id":"w1","BlockType":"WORD","Text":"Invoice"},
        {"Id":"w2","BlockType":"WORD","Text":"Total"},
        {"Id":"w3","BlockType":"WORD","Text":"12345"},
        {"Id":"w4","BlockType":"WORD","Text":"a|b"},
        {"Id":"tb","BlockType":"TABLE","Relationships":[
            {"Type":"CHILD","Ids":["c4","c1","c3","c2"]},
            {"Type":"TABLE_TITLE","Ids":["title"]}]},
        {"Id":"c1","BlockType":"CELL","RowIndex":1,"ColumnIndex":1,"Relationships":[{"Type":"CHILD","Ids":["w1"]}]},
        {"Id":"c2","BlockType":"CELL","RowIndex":1,"ColumnIndex":2,"Relationships":[{"Type":"CHILD","Ids":["w2"]}]},
        {"Id":"c3","BlockType":"CELL","RowIndex":2,"ColumnIndex":1,"Relationships":[{"Type":"CHILD","Ids":["w3"]}]},
        {"Id":"c4","BlockType":"CELL","RowIndex":2,"ColumnIndex":2,"Relationships":[{"Type":"CHILD","Ids":["w4"]}]},
        {"Id":"lt","BlockType":"LAYOUT_TABLE","Relationships":[{"Type":"CHILD","Ids":["l1","l2","l3","l4"]}]}]"#,
    vec![(0, "| Invoice | Total |\n| --- | --- |\n| 12345 | a\\|b |")]
)]
#[case::a_layout_table_that_links_its_table_renders_that_one(
    r#"[
        {"Id":"w1","BlockType":"WORD","Text":"first"},
        {"Id":"w2","BlockType":"WORD","Text":"second"},
        {"Id":"tb1","BlockType":"TABLE","Relationships":[{"Type":"CHILD","Ids":["c1"]}]},
        {"Id":"c1","BlockType":"CELL","RowIndex":1,"ColumnIndex":1,"Relationships":[{"Type":"CHILD","Ids":["w1"]}]},
        {"Id":"tb2","BlockType":"TABLE","Relationships":[{"Type":"CHILD","Ids":["c2"]}]},
        {"Id":"c2","BlockType":"CELL","RowIndex":1,"ColumnIndex":1,"Relationships":[{"Type":"CHILD","Ids":["w2"]}]},
        {"Id":"lt","BlockType":"LAYOUT_TABLE","Relationships":[{"Type":"CHILD","Ids":["tb2"]}]}]"#,
    vec![(0, "| second |\n| --- |")]
)]
#[case::a_missing_cell_leaves_an_empty_column(
    r#"[
        {"Id":"w1","BlockType":"WORD","Text":"a"},
        {"Id":"w2","BlockType":"WORD","Text":"b"},
        {"Id":"w3","BlockType":"WORD","Text":"c"},
        {"Id":"tb","BlockType":"TABLE","Relationships":[{"Type":"CHILD","Ids":["c1","c2","c3"]}]},
        {"Id":"c1","BlockType":"CELL","RowIndex":1,"ColumnIndex":1,"Relationships":[{"Type":"CHILD","Ids":["w1"]}]},
        {"Id":"c2","BlockType":"CELL","RowIndex":1,"ColumnIndex":2,"Relationships":[{"Type":"CHILD","Ids":["w2"]}]},
        {"Id":"c3","BlockType":"CELL","RowIndex":2,"ColumnIndex":2,"Relationships":[{"Type":"CHILD","Ids":["w3"]}]},
        {"Id":"lt","BlockType":"LAYOUT_TABLE","Relationships":[]}]"#,
    vec![(0, "| a | b |\n| --- | --- |\n|  | c |")]
)]
#[case::a_layout_table_without_table_blocks_keeps_its_lines(
    r#"[
        {"Id":"l1","BlockType":"LINE","Text":"Invoice Total"},
        {"Id":"l2","BlockType":"LINE","Text":"12345 67.89"},
        {"Id":"lt","BlockType":"LAYOUT_TABLE","Relationships":[{"Type":"CHILD","Ids":["l1","l2"]}]}]"#,
    vec![(0, "Invoice Total\n12345 67.89")]
)]
#[case::key_values_keep_one_line_each(
    r#"[
        {"Id":"l1","BlockType":"LINE","Text":"Name: Ana"},
        {"Id":"l2","BlockType":"LINE","Text":"Date: 2024-01-01"},
        {"Id":"kv","BlockType":"LAYOUT_KEY_VALUE","Relationships":[{"Type":"CHILD","Ids":["l1","l2"]}]}]"#,
    vec![(0, "Name: Ana\nDate: 2024-01-01")]
)]
#[case::a_figure_has_no_markdown(
    r#"[
        {"Id":"l1","BlockType":"LINE","Text":"Caption"},
        {"Id":"f","BlockType":"LAYOUT_FIGURE","Relationships":[]},
        {"Id":"p","BlockType":"LAYOUT_TEXT","Relationships":[{"Type":"CHILD","Ids":["l1"]}]}]"#,
    vec![(0, "Caption")]
)]
#[case::a_block_type_added_later_is_ignored(
    r#"[
        {"Id":"l1","BlockType":"LINE","Text":"Body"},
        {"Id":"new","BlockType":"LAYOUT_SIDEBAR","Relationships":[{"Type":"CHILD","Ids":["l1"]}]},
        {"Id":"p","BlockType":"LAYOUT_TEXT","Relationships":[{"Type":"CHILD","Ids":["l1"]}]}]"#,
    vec![(0, "Body")]
)]
#[case::a_table_cell_repeated_at_one_position_keeps_the_last(
    r#"[
        {"Id":"w1","BlockType":"WORD","Text":"first"},
        {"Id":"w2","BlockType":"WORD","Text":"second"},
        {"Id":"tb","BlockType":"TABLE","Relationships":[{"Type":"CHILD","Ids":["c1","c2"]}]},
        {"Id":"c1","BlockType":"CELL","RowIndex":1,"ColumnIndex":1,"Relationships":[{"Type":"CHILD","Ids":["w1"]}]},
        {"Id":"c2","BlockType":"CELL","RowIndex":1,"ColumnIndex":1,"Relationships":[{"Type":"CHILD","Ids":["w2"]}]},
        {"Id":"lt","BlockType":"LAYOUT_TABLE","Relationships":[{"Type":"CHILD","Ids":["tb"]}]}]"#,
    vec![(0, "| second |\n| --- |")]
)]
#[case::a_cell_without_its_column_is_skipped(
    r#"[
        {"Id":"w1","BlockType":"WORD","Text":"kept"},
        {"Id":"w2","BlockType":"WORD","Text":"dropped"},
        {"Id":"tb","BlockType":"TABLE","Relationships":[{"Type":"CHILD","Ids":["c1","c2"]}]},
        {"Id":"c1","BlockType":"CELL","RowIndex":1,"ColumnIndex":1,"Relationships":[{"Type":"CHILD","Ids":["w1"]}]},
        {"Id":"c2","BlockType":"CELL","RowIndex":2,"Relationships":[{"Type":"CHILD","Ids":["w2"]}]},
        {"Id":"lt","BlockType":"LAYOUT_TABLE","Relationships":[{"Type":"CHILD","Ids":["tb"]}]}]"#,
    vec![(0, "| kept |\n| --- |")]
)]
#[case::a_layout_block_without_a_page_belongs_to_page_one(
    r#"[
        {"Id":"l1","BlockType":"LINE","Text":"one"},
        {"Id":"p","BlockType":"LAYOUT_TEXT","Relationships":[{"Type":"CHILD","Ids":["l1"]}]},
        {"Id":"l2","BlockType":"LINE","Text":"two","Page":2},
        {"Id":"q","BlockType":"LAYOUT_TEXT","Page":2,"Relationships":[{"Type":"CHILD","Ids":["l2"]}]}]"#,
    vec![(0, "one"), (1, "two")]
)]
#[case::a_child_without_text_adds_nothing(
    r#"[
        {"Id":"l1","BlockType":"LINE","Text":"Body"},
        {"Id":"empty","BlockType":"PAGE"},
        {"Id":"p","BlockType":"LAYOUT_TEXT","Relationships":[{"Type":"CHILD","Ids":["l1","empty"]}]}]"#,
    vec![(0, "Body")]
)]
#[case::without_layout_blocks_lines_are_used(
    r#"[
        {"Id":"l1","BlockType":"LINE","Text":"first"},
        {"Id":"w1","BlockType":"WORD","Text":"first"},
        {"Id":"l2","BlockType":"LINE","Text":"second"}]"#,
    vec![(0, "first\nsecond")]
)]
#[case::list_items_lose_their_own_bullet(
    r#"[
        {"Id":"l1","BlockType":"LINE","Text":"- item"},
        {"Id":"l2","BlockType":"LINE","Text":"* item"},
        {"Id":"l3","BlockType":"LINE","Text":"\u2022 item"},
        {"Id":"l4","BlockType":"LINE","Text":"\u00b7item"},
        {"Id":"l5","BlockType":"LINE","Text":"item - with a dash"},
        {"Id":"i1","BlockType":"LAYOUT_TEXT","Relationships":[{"Type":"CHILD","Ids":["l1"]}]},
        {"Id":"i2","BlockType":"LAYOUT_TEXT","Relationships":[{"Type":"CHILD","Ids":["l2"]}]},
        {"Id":"i3","BlockType":"LAYOUT_TEXT","Relationships":[{"Type":"CHILD","Ids":["l3"]}]},
        {"Id":"i4","BlockType":"LAYOUT_TEXT","Relationships":[{"Type":"CHILD","Ids":["l4"]}]},
        {"Id":"i5","BlockType":"LAYOUT_TEXT","Relationships":[{"Type":"CHILD","Ids":["l5"]}]},
        {"Id":"ul","BlockType":"LAYOUT_LIST","Relationships":[{"Type":"CHILD","Ids":["i1","i2","i3","i4","i5"]}]}]"#,
    vec![(0, "- item\n- item\n- item\n- item\n- item - with a dash")]
)]
#[case::each_page_gets_its_own_markdown_and_its_own_tables(
    r#"[
        {"Id":"a","BlockType":"LINE","Text":"one","Page":1},
        {"Id":"b","BlockType":"LINE","Text":"two","Page":2},
        {"Id":"w","BlockType":"WORD","Text":"cell","Page":2},
        {"Id":"t1","BlockType":"LAYOUT_TEXT","Page":1,"Relationships":[{"Type":"CHILD","Ids":["a"]}]},
        {"Id":"tb","BlockType":"TABLE","Page":2,"Relationships":[{"Type":"CHILD","Ids":["c"]}]},
        {"Id":"c","BlockType":"CELL","Page":2,"RowIndex":1,"ColumnIndex":1,"Relationships":[{"Type":"CHILD","Ids":["w"]}]},
        {"Id":"lt","BlockType":"LAYOUT_TABLE","Page":2,"Relationships":[{"Type":"CHILD","Ids":["b"]}]}]"#,
    vec![(0, "one"), (1, "| cell |\n| --- |")]
)]
fn blocks_become_markdown_pages(#[case] blocks: &str, #[case] expected: Vec<(i64, &str)>) {
    let response = analyze(blocks).expect("the Textract answer decodes");

    assert_eq!(pages(&response), expected);
    assert_eq!(response.model, MODEL);
}

#[test]
fn analyze_usage_reports_the_document_page_count() {
    let response = analyze(r#"[{"BlockType":"LINE","Text":"only"}]"#).expect("the answer decodes");

    assert_eq!(response.usage_info.unwrap().pages_processed, Some(1));
}

#[test]
fn a_cyclic_block_graph_is_refused_instead_of_nesting_forever() {
    let error = analyze(
        r#"[
            {"Id":"a","BlockType":"LAYOUT_TEXT","Relationships":[{"Type":"CHILD","Ids":["b"]}]},
            {"Id":"b","BlockType":"LAYOUT_TEXT","Relationships":[{"Type":"CHILD","Ids":["a"]}]}]"#,
    )
    .unwrap_err();

    assert!(
        matches!(&error, OcrError::InvalidResponse(message) if message.contains("nest")),
        "{error}"
    );
}

#[rstest]
#[case::at_the_limit(1000, true)]
#[case::over_the_limit(1001, false)]
fn table_columns_are_limited(#[case] column: usize, #[case] allowed: bool) {
    let blocks = format!(
        r#"[{{"Id":"w","BlockType":"WORD","Text":"x"}},
            {{"Id":"tb","BlockType":"TABLE","Relationships":[{{"Type":"CHILD","Ids":["c"]}}]}},
            {{"Id":"c","BlockType":"CELL","RowIndex":1,"ColumnIndex":{column},"Relationships":[{{"Type":"CHILD","Ids":["w"]}}]}},
            {{"Id":"lt","BlockType":"LAYOUT_TABLE","Relationships":[{{"Type":"CHILD","Ids":["tb"]}}]}}]"#
    );

    let response = analyze(&blocks);

    assert_eq!(response.is_ok(), allowed, "{response:?}");
    if !allowed {
        assert!(
            matches!(&response, Err(OcrError::NumericRange("table columns"))),
            "{response:?}"
        );
    }
}

fn walk(blocks: &str, limit: usize) -> Result<String, OcrError> {
    let raw = format!(r#"{{"Blocks":{blocks}}}"#);
    let parsed: TextractResponse = serde_json::from_str(&raw)
        .map_err(|_source| OcrError::InvalidResponse("the test blocks parse".to_owned()))?;
    let by_id: HashMap<&str, &Block> = parsed
        .blocks
        .iter()
        .map(|block| (block.id.as_str(), block))
        .collect();
    let mut work = WorkBudget { spent: 0, limit };
    layout_markdown(&parsed.blocks, 1, &by_id, &mut work)
}

#[rstest]
#[case::exactly_at_the_limit(4, true)]
#[case::one_below_the_work(3, false)]
fn the_work_budget_is_enforced(#[case] limit: usize, #[case] allowed: bool) {
    let walked = walk(
        r#"[{"Id":"root","BlockType":"LAYOUT_TEXT","Text":"abc"}]"#,
        limit,
    );

    assert_eq!(walked.is_ok(), allowed, "limit {limit}");
    if allowed {
        assert_eq!(walked.expect("the walk fits the budget"), "abc");
    } else {
        assert!(
            matches!(&walked, Err(OcrError::InvalidResponse(message)) if message.contains("work")),
            "limit {limit}"
        );
    }
}

#[test]
fn a_fan_out_block_graph_is_refused() {
    // 18 levels keep the uncharged walk finite, so a broken budget fails instead of hanging
    let walked = walk(&fan_out_blocks(18), 1_000);

    assert!(
        matches!(&walked, Err(OcrError::InvalidResponse(message)) if message.contains("work")),
        "the fan out graph is refused"
    );
}

#[test]
fn a_repeated_large_text_block_is_refused() {
    let ids = repeat_n("\"line\"", 100).collect::<Vec<&str>>().join(",");
    let text = "x".repeat(1_000);
    let blocks = format!(
        r#"[{{"Id":"root","BlockType":"LAYOUT_TEXT","Relationships":[{{"Type":"CHILD","Ids":[{ids}]}}]}},{{"Id":"line","BlockType":"LINE","Text":"{text}"}}]"#
    );

    let walked = walk(&blocks, 10_000);

    assert!(
        matches!(&walked, Err(OcrError::InvalidResponse(message)) if message.contains("work")),
        "the repeated text block is refused"
    );
}

#[test]
fn a_repeated_child_with_many_dangling_ids_is_refused() {
    let parents = repeat_n("\"parent\"", 100).collect::<Vec<&str>>().join(",");
    let dangling = repeat_n("\"missing\"", 200)
        .collect::<Vec<&str>>()
        .join(",");
    let blocks = format!(
        r#"[{{"Id":"root","BlockType":"LAYOUT_TEXT","Relationships":[{{"Type":"CHILD","Ids":[{parents}]}}]}},{{"Id":"parent","BlockType":"PAGE","Relationships":[{{"Type":"CHILD","Ids":[{dangling}]}}]}}]"#
    );

    let walked = walk(&blocks, 10_000);

    assert!(
        matches!(&walked, Err(OcrError::InvalidResponse(message)) if message.contains("work")),
        "the repeated dangling ids are refused"
    );
}

#[test]
fn a_repeated_table_scan_is_refused() {
    let walked = walk(&repeated_table_blocks(20, 50), 100);

    assert!(
        matches!(&walked, Err(OcrError::InvalidResponse(message)) if message.contains("work")),
        "the table scan is charged by the walk"
    );
}

fn repeated_table_blocks(renderers: usize, dangling: usize) -> String {
    let mut blocks = Vec::new();
    for index in 0..renderers {
        blocks.push(format!(
            r#"{{"Id":"lt{index}","BlockType":"LAYOUT_TABLE","Relationships":[{{"Type":"CHILD","Ids":["table"]}}]}}"#
        ));
    }
    let ids = repeat_n("\"missing\"", dangling)
        .collect::<Vec<&str>>()
        .join(",");
    blocks.push(format!(
        r#"{{"Id":"table","BlockType":"TABLE","Relationships":[{{"Type":"CHILD","Ids":[{ids}]}}]}}"#
    ));
    format!("[{}]", blocks.join(","))
}

#[test]
fn work_over_the_response_size_is_refused() {
    let ids = repeat_n("\"line\"", 100).collect::<Vec<&str>>().join(",");
    let text = "x".repeat(MEBIBYTE);
    let blocks = format!(
        r#"[{{"Id":"root","BlockType":"LAYOUT_TEXT","Relationships":[{{"Type":"CHILD","Ids":[{ids}]}}]}},{{"Id":"line","BlockType":"LINE","Text":"{text}"}}]"#
    );

    let response = analyze(&blocks);

    assert!(
        matches!(&response, Err(OcrError::InvalidResponse(message)) if message.contains("work")),
        "the body is under the response cap and the work is over it"
    );
}

#[test]
fn one_budget_covers_every_page() {
    let ids = repeat_n("\"line\"", 40).collect::<Vec<&str>>().join(",");
    let text = "x".repeat(MEBIBYTE);
    let blocks = format!(
        r#"[{{"Id":"first","BlockType":"LAYOUT_TEXT","Relationships":[{{"Type":"CHILD","Ids":[{ids}]}}]}},{{"Id":"second","BlockType":"LAYOUT_TEXT","Page":2,"Relationships":[{{"Type":"CHILD","Ids":[{ids}]}}]}},{{"Id":"line","BlockType":"LINE","Text":"{text}"}}]"#
    );

    let response = analyze(&blocks);

    assert!(
        matches!(&response, Err(OcrError::InvalidResponse(message)) if message.contains("work")),
        "the pages share one budget"
    );
}

fn fan_out_blocks(levels: usize) -> String {
    let mut blocks = Vec::new();
    for level in 0..levels {
        let next = level.saturating_add(1);
        blocks.push(format!(
            r#"{{"Id":"b{level}","BlockType":"LAYOUT_TEXT","Relationships":[{{"Type":"CHILD","Ids":["b{next}","b{next}"]}}]}}"#
        ));
    }
    blocks.push(format!(
        r#"{{"Id":"b{levels}","BlockType":"LINE","Text":"x"}}"#
    ));
    format!("[{}]", blocks.join(","))
}

#[rstest]
#[case::text_at_the_limit(64, true)]
#[case::a_link_block_past_the_limit(65, false)]
fn block_nesting_is_limited(#[case] depth: usize, #[case] allowed: bool) {
    let response = analyze(&nested_blocks(depth));

    assert_eq!(response.is_ok(), allowed, "{response:?}");
    if allowed {
        assert_eq!(
            pages(&response.expect("the answer decodes")),
            vec![(0, "deep")]
        );
    }
}

fn nested_blocks(depth: usize) -> String {
    let mut blocks = Vec::new();
    for level in 0..depth {
        let kind = if level == 0 { "LAYOUT_TEXT" } else { "PAGE" };
        blocks.push(format!(
            r#"{{"Id":"b{level}","BlockType":"{kind}","Relationships":[{{"Type":"CHILD","Ids":["b{}"]}}]}}"#,
            level.saturating_add(1)
        ));
    }
    blocks.push(format!(
        r#"{{"Id":"b{depth}","BlockType":"LINE","Text":"deep"}}"#
    ));
    format!("[{}]", blocks.join(","))
}

#[test]
fn a_body_that_is_not_textract_json_is_an_invalid_response() {
    let error = TextractAnalyzeDocumentConfig
        .transform_ocr_response(MODEL, b"<html>bad gateway</html>", RequestFormat::Litellm)
        .unwrap_err();

    assert!(matches!(&error, OcrError::ResponseField { .. }), "{error}");
}
