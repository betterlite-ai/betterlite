use betterlite_config::sections::cisco_verdict_contracts::InspectionType;

use crate::cisco_conversation::Fields;
use crate::cisco_verdict::Action;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Input,
    Output,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScanContext {
    pub surface: InspectionType,
    pub direction: Direction,
}

#[derive(Debug, Clone)]
pub struct CiscoVerdict {
    pub is_safe: Option<bool>,
    pub classifications: Vec<String>,
    pub severity: Option<String>,
    pub rules: Vec<Fields>,
    pub explanation: Option<String>,
    pub event_id: Option<String>,
    pub action: Option<Action>,
    pub sanitized_text: Option<String>,
    pub sanitized_messages: Option<Vec<Fields>>,
    pub sanitized_mcp_arguments: Option<Fields>,
}

mod tests;
