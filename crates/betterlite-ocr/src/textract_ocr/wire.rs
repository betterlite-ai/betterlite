use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize)]
pub struct TextractDocument {
    #[serde(rename = "Bytes")]
    pub(crate) bytes: String,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum FeatureType {
    Tables,
    Forms,
    Queries,
    Signatures,
    Layout,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub(crate) enum BlockType {
    KeyValueSet,
    Page,
    Line,
    Word,
    Table,
    Cell,
    SelectionElement,
    MergedCell,
    Title,
    Query,
    QueryResult,
    Signature,
    TableTitle,
    TableFooter,
    LayoutText,
    LayoutTitle,
    LayoutHeader,
    LayoutFooter,
    LayoutSectionHeader,
    LayoutPageNumber,
    LayoutList,
    LayoutFigure,
    LayoutTable,
    LayoutKeyValue,
    #[serde(other)]
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LayoutType {
    Text,
    Title,
    Header,
    Footer,
    SectionHeader,
    PageNumber,
    List,
    Figure,
    Table,
    KeyValue,
}

impl BlockType {
    pub(crate) const fn layout(self) -> Option<LayoutType> {
        match self {
            Self::LayoutText => Some(LayoutType::Text),
            Self::LayoutTitle => Some(LayoutType::Title),
            Self::LayoutHeader => Some(LayoutType::Header),
            Self::LayoutFooter => Some(LayoutType::Footer),
            Self::LayoutSectionHeader => Some(LayoutType::SectionHeader),
            Self::LayoutPageNumber => Some(LayoutType::PageNumber),
            Self::LayoutList => Some(LayoutType::List),
            Self::LayoutFigure => Some(LayoutType::Figure),
            Self::LayoutTable => Some(LayoutType::Table),
            Self::LayoutKeyValue => Some(LayoutType::KeyValue),
            Self::KeyValueSet
            | Self::Page
            | Self::Line
            | Self::Word
            | Self::Table
            | Self::Cell
            | Self::SelectionElement
            | Self::MergedCell
            | Self::Title
            | Self::Query
            | Self::QueryResult
            | Self::Signature
            | Self::TableTitle
            | Self::TableFooter
            | Self::Unknown => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub(crate) enum RelationshipType {
    Value,
    Child,
    ComplexFeatures,
    MergedCell,
    Title,
    Answer,
    Table,
    TableTitle,
    TableFooter,
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub(crate) struct Block {
    #[serde(default)]
    pub(crate) id: String,
    #[serde(rename = "BlockType")]
    pub(crate) kind: BlockType,
    pub(crate) text: Option<String>,
    #[serde(default = "default_page")]
    pub(crate) page: i64,
    pub(crate) row_index: Option<usize>,
    pub(crate) column_index: Option<usize>,
    #[serde(default)]
    pub(crate) relationships: Vec<Relationship>,
}

const fn default_page() -> i64 {
    1
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub(crate) struct Relationship {
    #[serde(rename = "Type")]
    pub(crate) kind: RelationshipType,
    #[serde(default)]
    pub(crate) ids: Vec<String>,
}

impl Block {
    pub(crate) fn children(&self) -> impl Iterator<Item = &str> {
        self.relationships
            .iter()
            .filter(|relationship| relationship.kind == RelationshipType::Child)
            .flat_map(|relationship| relationship.ids.iter().map(String::as_str))
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub(crate) struct DocumentMetadata {
    pub(crate) pages: Option<i64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub(crate) struct TextractResponse {
    #[serde(default)]
    pub(crate) blocks: Vec<Block>,
    pub(crate) document_metadata: Option<DocumentMetadata>,
}
