use std::collections::BTreeSet;

use indexmap::IndexMap;

use super::ConfigFileError;

const DEFAULT_SECTION: &str = "DEFAULT";

pub(super) struct Scan {
    pub(super) defaults: IndexMap<String, Vec<String>>,
    pub(super) sections: Vec<(String, IndexMap<String, Vec<String>>)>,
}

enum Open {
    None,
    Defaults,
    Named(usize),
}

fn split_lines(contents: &str) -> Vec<&str> {
    let mut lines = Vec::new();
    let mut start = 0_usize;
    let mut chars = contents.char_indices().peekable();
    while let Some((index, character)) = chars.next() {
        match character {
            '\n' => {
                lines.push(contents.get(start..index).unwrap_or_default());
                start = index.saturating_add(1);
            }
            '\r' => {
                lines.push(contents.get(start..index).unwrap_or_default());
                start = if let Some(&(newline_index, '\n')) = chars.peek() {
                    chars.next();
                    newline_index.saturating_add(1)
                } else {
                    index.saturating_add(1)
                };
            }
            _ => {}
        }
    }
    if start < contents.len() {
        lines.push(contents.get(start..).unwrap_or_default());
    }
    lines
}

fn parse_header(stripped: &str) -> Option<&str> {
    let after_bracket = stripped.strip_prefix('[')?;
    let last_bracket = after_bracket.rfind(']')?;
    if last_bracket == 0 {
        return None;
    }
    after_bracket.get(..last_bracket)
}

fn split_option(stripped: &str) -> Option<(&str, &str)> {
    let equals = stripped.find('=');
    let colon = stripped.find(':');
    let position = match (equals, colon) {
        (Some(equals_at), Some(colon_at)) => Some(equals_at.min(colon_at)),
        (Some(equals_at), None) => Some(equals_at),
        (None, Some(colon_at)) => Some(colon_at),
        (None, None) => None,
    }?;
    let key = stripped.get(..position)?;
    let value_start = position.checked_add(1)?;
    let value = stripped.get(value_start..)?;
    Some((key, value))
}

struct ScanState {
    defaults: IndexMap<String, Vec<String>>,
    sections: Vec<(String, IndexMap<String, Vec<String>>)>,
    seen_sections: BTreeSet<String>,
    seen_options: BTreeSet<(String, String)>,
    current: Open,
    current_option: Option<String>,
    indent_level: usize,
    deferred: Option<ConfigFileError>,
}

impl Default for ScanState {
    fn default() -> Self {
        Self {
            defaults: IndexMap::new(),
            sections: Vec::new(),
            seen_sections: BTreeSet::new(),
            seen_options: BTreeSet::new(),
            current: Open::None,
            current_option: None,
            indent_level: 0,
            deferred: None,
        }
    }
}

impl ScanState {
    fn finish(self) -> Result<Scan, ConfigFileError> {
        if let Some(error) = self.deferred {
            return Err(error);
        }
        Ok(Scan {
            defaults: self.defaults,
            sections: self.sections,
        })
    }

    fn current_section_name(&self) -> Option<String> {
        match &self.current {
            Open::None => None,
            Open::Defaults => Some(DEFAULT_SECTION.to_owned()),
            Open::Named(position) => self.sections.get(*position).map(|(name, _)| name.clone()),
        }
    }

    fn active_values(&mut self) -> Option<&mut IndexMap<String, Vec<String>>> {
        match &self.current {
            Open::None => None,
            Open::Defaults => Some(&mut self.defaults),
            Open::Named(position) => self.sections.get_mut(*position).map(|(_, values)| values),
        }
    }

    fn continue_current_option(&mut self, text: &str) {
        if let Some(option) = self.current_option.clone()
            && let Some(values) = self.active_values()
            && let Some(lines) = values.get_mut(&option)
        {
            lines.push(text.to_owned());
        }
    }

    fn open_default(&mut self) {
        self.current = Open::Defaults;
        self.current_option = None;
    }

    fn open_named(&mut self, name: String, line: usize) -> Result<(), ConfigFileError> {
        if self.seen_sections.contains(&name) {
            return Err(ConfigFileError::DuplicateSection {
                section: name,
                line,
            });
        }
        self.seen_sections.insert(name.clone());
        self.sections.push((name, IndexMap::new()));
        self.current = Open::Named(self.sections.len().saturating_sub(1));
        self.current_option = None;
        Ok(())
    }

    fn record_option(&mut self, key_lower: String, value: String, section_name: String) {
        self.seen_options.insert((section_name, key_lower.clone()));
        let entry = key_lower.clone();
        if let Some(values) = self.active_values() {
            values.insert(entry, vec![value]);
        }
        self.current_option = Some(key_lower);
    }

    fn handle_header(&mut self, header: &str, line: usize) -> Result<(), ConfigFileError> {
        if header == DEFAULT_SECTION {
            self.open_default();
            return Ok(());
        }
        self.open_named(header.to_owned(), line)
    }

    fn handle_option_line(&mut self, stripped: &str, line: usize) -> Result<(), ConfigFileError> {
        let Some(section_name) = self.current_section_name() else {
            return Err(ConfigFileError::BeforeHeader { line });
        };
        let Some((key, value)) = split_option(stripped) else {
            self.deferred
                .get_or_insert(ConfigFileError::MissingDelimiter {
                    section: section_name,
                    line,
                });
            return Ok(());
        };
        let key_lower = key.trim_end().to_lowercase();
        if key_lower.is_empty() {
            self.deferred.get_or_insert(ConfigFileError::EmptyKey {
                section: section_name,
                line,
            });
            self.current_option = None;
            return Ok(());
        }
        if let Some(duplicate) = self.duplicate_option_error(&section_name, &key_lower, line) {
            return Err(duplicate);
        }
        self.record_option(key_lower, value.trim().to_owned(), section_name);
        Ok(())
    }

    fn duplicate_option_error(
        &self,
        section_name: &str,
        key_lower: &str,
        line: usize,
    ) -> Option<ConfigFileError> {
        let identity = (section_name.to_owned(), key_lower.to_owned());
        self.seen_options
            .contains(&identity)
            .then(|| ConfigFileError::DuplicateOption {
                section: section_name.to_owned(),
                option: key_lower.to_owned(),
                line,
            })
    }

    fn handle_line(&mut self, raw_line: &str, line: usize) -> Result<(), ConfigFileError> {
        let stripped = raw_line.trim();
        if stripped.starts_with('#') || stripped.starts_with(';') {
            return Ok(());
        }
        if stripped.is_empty() {
            self.continue_current_option("");
            return Ok(());
        }
        let leading = raw_line
            .chars()
            .take_while(|character| character.is_whitespace())
            .count();
        if self.current_option.is_some() && leading > self.indent_level {
            self.continue_current_option(stripped);
            return Ok(());
        }
        self.indent_level = leading;
        if let Some(header) = parse_header(stripped) {
            return self.handle_header(header, line);
        }
        self.handle_option_line(stripped, line)
    }
}

pub(super) fn scan(contents: &str) -> Result<Scan, ConfigFileError> {
    let mut state = ScanState::default();
    for (line_index, raw_line) in split_lines(contents).into_iter().enumerate() {
        let line = line_index.saturating_add(1);
        state.handle_line(raw_line, line)?;
    }
    state.finish()
}
