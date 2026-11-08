use std::collections::BTreeMap;
use std::mem;
use std::str::Chars;

#[derive(Debug, Clone)]
pub(super) enum Value {
    Scalar(String),
    Nested,
}

#[derive(Debug)]
pub(super) struct Section {
    pub(super) name: String,
    pub(super) values: BTreeMap<String, Value>,
}

struct RawSection {
    name: String,
    values: BTreeMap<String, Vec<String>>,
}

pub(super) fn parse(source: &str) -> Result<Vec<Section>, ()> {
    let normalized = source.replace("\r\n", "\n").replace('\r', "\n");
    let mut parser = IniParser {
        sections: Vec::new(),
        current: None,
        option: None,
        indent_level: 0,
    };
    for line in normalized.split('\n') {
        parser.line(line)?;
    }
    parser.finish()
}

struct IniParser {
    sections: Vec<RawSection>,
    current: Option<usize>,
    option: Option<String>,
    indent_level: usize,
}

impl IniParser {
    fn line(&mut self, raw: &str) -> Result<(), ()> {
        let clean = clean_line(raw);
        if clean.is_empty() {
            return Ok(());
        }
        let indent = raw.chars().take_while(|ch| ch.is_whitespace()).count();
        if self.continuation(clean, indent) {
            return Ok(());
        }
        self.indent_level = indent;
        if let Some(name) = section_header(clean) {
            return self.start_section(name);
        }
        self.start_option(clean)
    }

    fn continuation(&mut self, clean: &str, indent: usize) -> bool {
        if indent <= self.indent_level {
            return false;
        }
        let (Some(index), Some(option)) = (self.current, self.option.clone()) else {
            return false;
        };
        let Some(section) = self.sections.get_mut(index) else {
            return false;
        };
        let Some(lines) = section.values.get_mut(&option) else {
            return false;
        };
        lines.push(clean.to_owned());
        true
    }

    fn start_section(&mut self, name: &str) -> Result<(), ()> {
        match name {
            "DEFAULT" => {
                // configparser keeps one defaults mapping, so a repeated [DEFAULT] merges into it
                if let Some(index) = self
                    .sections
                    .iter()
                    .position(|section| section.name == "DEFAULT")
                {
                    self.current = Some(index);
                    self.option = None;
                    return Ok(());
                }
            }
            _ => {
                if self.sections.iter().any(|section| section.name == name) {
                    return Err(());
                }
            }
        }
        self.sections.push(RawSection {
            name: name.to_owned(),
            values: BTreeMap::new(),
        });
        let Some(index) = self.sections.len().checked_sub(1) else {
            return Err(());
        };
        self.current = Some(index);
        self.option = None;
        Ok(())
    }

    fn start_option(&mut self, clean: &str) -> Result<(), ()> {
        let Some(index) = self.current else {
            return Err(());
        };
        let Some((key, value)) = option_line(clean) else {
            return Err(());
        };
        let normalized = key.trim_end().to_lowercase();
        if normalized.is_empty() {
            return Err(());
        }
        let Some(section) = self.sections.get_mut(index) else {
            return Err(());
        };
        if section.values.contains_key(&normalized) {
            return Err(());
        }
        section
            .values
            .insert(normalized.clone(), vec![value.to_owned()]);
        self.option = Some(normalized);
        Ok(())
    }

    fn finish(self) -> Result<Vec<Section>, ()> {
        let mut sections = Vec::with_capacity(self.sections.len());
        for section in self.sections {
            let mut values = BTreeMap::new();
            for (key, lines) in section.values {
                values.insert(key, parse_value(&lines)?);
            }
            sections.push(Section {
                name: section.name,
                values,
            });
        }
        Ok(apply_defaults(sections))
    }
}

fn clean_line(raw: &str) -> &str {
    let trimmed = raw.trim();
    if trimmed.starts_with('#') || trimmed.starts_with(';') {
        return "";
    }
    trimmed
}

fn section_header(clean: &str) -> Option<&str> {
    let rest = clean.strip_prefix('[')?;
    let end = rest.rfind(']')?;
    let header = rest.get(..end)?;
    (!header.is_empty()).then_some(header)
}

fn option_line(clean: &str) -> Option<(&str, &str)> {
    let position = clean.find(['=', ':'])?;
    let key = clean.get(..position)?;
    let value = clean.get(position..)?.get(1..)?;
    Some((key.trim_end(), value.trim()))
}

fn parse_value(lines: &[String]) -> Result<Value, ()> {
    let joined = lines.join("\n");
    let text = joined.trim_end();
    if !text.starts_with('\n') {
        return Ok(Value::Scalar(text.to_owned()));
    }
    for line in text.split('\n') {
        let stripped = line.trim();
        if stripped.is_empty() {
            continue;
        }
        if stripped.split_once('=').is_none() {
            return Err(());
        }
    }
    Ok(Value::Nested)
}

fn apply_defaults(sections: Vec<Section>) -> Vec<Section> {
    let defaults = sections
        .iter()
        .find(|section| section.name == "DEFAULT")
        .map(|section| section.values.clone());
    let mut merged = Vec::with_capacity(sections.len());
    for mut section in sections {
        if section.name != "DEFAULT"
            && let Some(default_values) = &defaults
        {
            for (key, value) in default_values {
                section
                    .values
                    .entry(key.clone())
                    .or_insert_with(|| value.clone());
            }
        }
        merged.push(section);
    }
    merged
}

pub(super) fn shell_split(text: &str) -> Option<Vec<String>> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut started = false;
    let mut chars = text.chars();
    while let Some(ch) = chars.next() {
        if ch.is_whitespace() {
            if started {
                tokens.push(mem::take(&mut current));
                started = false;
            }
            continue;
        }
        started = true;
        match ch {
            '\'' => read_single_quoted(&mut chars, &mut current)?,
            '"' => read_double_quoted(&mut chars, &mut current)?,
            '\\' => current.push(chars.next()?),
            other => current.push(other),
        }
    }
    if started {
        tokens.push(current);
    }
    Some(tokens)
}

fn read_single_quoted(chars: &mut Chars<'_>, current: &mut String) -> Option<()> {
    loop {
        match chars.next()? {
            '\'' => return Some(()),
            inner => current.push(inner),
        }
    }
}

fn read_double_quoted(chars: &mut Chars<'_>, current: &mut String) -> Option<()> {
    loop {
        match chars.next()? {
            '"' => return Some(()),
            '\\' => current.push(chars.next()?),
            inner => current.push(inner),
        }
    }
}
