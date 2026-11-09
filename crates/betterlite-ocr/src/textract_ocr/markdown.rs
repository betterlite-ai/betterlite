use std::collections::{BTreeMap, BTreeSet, HashMap};

use crate::ocr_error::OcrError;
use crate::textract_ocr::wire::{Block, BlockType, LayoutType};

const MAX_BLOCK_DEPTH: usize = 64;
const MAX_TABLE_COLUMNS: usize = 1000;
const BULLETS: [char; 4] = ['-', '*', '\u{2022}', '\u{00b7}'];

pub(crate) struct WorkBudget {
    pub(crate) spent: usize,
    pub(crate) limit: usize,
}

impl WorkBudget {
    fn charge(&mut self, amount: usize) -> Result<(), OcrError> {
        self.spent = self.spent.saturating_add(amount);
        if self.spent > self.limit {
            return Err(OcrError::InvalidResponse(
                "the Textract blocks need more work than the response size allows".to_owned(),
            ));
        }
        Ok(())
    }
}

struct PageTables<'page> {
    blocks: Vec<&'page Block>,
    ordinal: HashMap<&'page str, usize>,
}

// Layout blocks arrive in reading order; a list's items are repeated as top-level LAYOUT_TEXT blocks
pub(crate) fn layout_markdown(
    blocks: &[Block],
    page: i64,
    by_id: &HashMap<&str, &Block>,
    work: &mut WorkBudget,
) -> Result<String, OcrError> {
    let on_page: Vec<&Block> = blocks.iter().filter(|block| block.page == page).collect();
    let list_items: BTreeSet<&str> = on_page
        .iter()
        .filter(|block| block.kind == BlockType::LayoutList)
        .flat_map(|block| block.children())
        .collect();
    let tables = PageTables {
        blocks: on_page
            .iter()
            .filter(|block| block.kind == BlockType::Table)
            .copied()
            .collect(),
        ordinal: on_page
            .iter()
            .filter(|block| block.kind == BlockType::LayoutTable)
            .enumerate()
            .map(|(ordinal, block)| (block.id.as_str(), ordinal))
            .collect(),
    };
    let mut sections = Vec::new();
    for block in on_page
        .iter()
        .filter(|block| !list_items.contains(block.id.as_str()))
    {
        let Some(layout) = block.kind.layout() else {
            continue;
        };
        let section = section_markdown(block, layout, by_id, &tables, work)?;
        if !section.trim().is_empty() {
            sections.push(section);
        }
    }
    Ok(sections.join("\n\n"))
}

fn section_markdown(
    block: &Block,
    layout: LayoutType,
    by_id: &HashMap<&str, &Block>,
    tables: &PageTables<'_>,
    work: &mut WorkBudget,
) -> Result<String, OcrError> {
    match layout {
        LayoutType::Title => Ok(format!("# {}", text_of(block, by_id, " ", 0, work)?)),
        LayoutType::SectionHeader => Ok(format!("## {}", text_of(block, by_id, " ", 0, work)?)),
        LayoutType::List => list_markdown(block, by_id, work),
        LayoutType::Table => {
            let linked = block
                .children()
                .filter_map(|id| by_id.get(id).copied())
                .find(|child| child.kind == BlockType::Table);
            let positioned = tables
                .ordinal
                .get(block.id.as_str())
                .and_then(|ordinal| tables.blocks.get(*ordinal).copied());
            let Some(table) = linked.or(positioned) else {
                return text_of(block, by_id, "\n", 0, work);
            };
            table_markdown(table, by_id, work)
        }
        LayoutType::KeyValue => text_of(block, by_id, "\n", 0, work),
        LayoutType::Figure => Ok(String::new()),
        LayoutType::Text | LayoutType::Header | LayoutType::Footer | LayoutType::PageNumber => {
            text_of(block, by_id, " ", 0, work)
        }
    }
}

fn list_markdown(
    block: &Block,
    by_id: &HashMap<&str, &Block>,
    work: &mut WorkBudget,
) -> Result<String, OcrError> {
    let mut items = Vec::new();
    for item in block.children().filter_map(|id| by_id.get(id).copied()) {
        let text = text_of(item, by_id, " ", 0, work)?;
        items.push(format!(
            "- {}",
            text.trim_start_matches(BULLETS).trim_start()
        ));
    }
    Ok(items.join("\n"))
}

// one visit, its text bytes, and the child ids it lists are all work the walk does
fn charge_block(block: &Block, work: &mut WorkBudget) -> Result<(), OcrError> {
    work.charge(
        1_usize
            .saturating_add(block.text.as_ref().map_or(0, String::len))
            .saturating_add(block.children().count()),
    )
}

fn text_of(
    block: &Block,
    by_id: &HashMap<&str, &Block>,
    separator: &str,
    depth: usize,
    work: &mut WorkBudget,
) -> Result<String, OcrError> {
    charge_block(block, work)?;
    if let Some(text) = &block.text {
        return Ok(text.clone());
    }
    if depth >= MAX_BLOCK_DEPTH {
        return Err(OcrError::InvalidResponse(
            "the Textract blocks nest deeper than the allowed limit".to_owned(),
        ));
    }
    let mut texts = Vec::new();
    for child in block.children().filter_map(|id| by_id.get(id).copied()) {
        let text = text_of(child, by_id, separator, depth.saturating_add(1), work)?;
        if !text.is_empty() {
            texts.push(text);
        }
    }
    Ok(texts.join(separator))
}

fn table_markdown(
    table: &Block,
    by_id: &HashMap<&str, &Block>,
    work: &mut WorkBudget,
) -> Result<String, OcrError> {
    charge_block(table, work)?;
    let mut cells: BTreeMap<(usize, usize), String> = BTreeMap::new();
    for cell in table.children().filter_map(|id| by_id.get(id).copied()) {
        if cell.kind != BlockType::Cell {
            continue;
        }
        let (Some(row), Some(column)) = (cell.row_index, cell.column_index) else {
            continue;
        };
        let text = text_of(cell, by_id, " ", 0, work)?.replace('|', "\\|");
        let _previous = cells.insert((row, column), text);
    }
    let columns = cells.keys().map(|(_, column)| *column).max().unwrap_or(0);
    if columns > MAX_TABLE_COLUMNS {
        return Err(OcrError::NumericRange("table columns"));
    }
    let rows: BTreeSet<usize> = cells.keys().map(|(row, _)| *row).collect();
    let divider = format!("|{}", " --- |".repeat(columns));
    let mut lines = Vec::new();
    for (position, row) in rows.iter().enumerate() {
        let values: Vec<&str> = (1..=columns)
            .map(|column| cells.get(&(*row, column)).map_or("", String::as_str))
            .collect();
        lines.push(format!("| {} |", values.join(" | ")));
        if position == 0 {
            lines.push(divider.clone());
        }
    }
    Ok(lines.join("\n"))
}
