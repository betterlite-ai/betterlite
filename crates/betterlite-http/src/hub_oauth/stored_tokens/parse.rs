use std::str::{self, Utf8Error};

use secrecy::{ExposeSecret as _, SecretString};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ParseError {
    #[error("the stored tokens file is not UTF-8")]
    Utf8(#[source] Utf8Error),
    #[error("line {line} comes before any section header")]
    MissingSectionHeader { line: usize },
    #[error("line {line} opens the section [{name}] a second time")]
    DuplicateSection { line: usize, name: String },
    #[error("line {line} repeats the option {name} of [{section}]")]
    DuplicateOption {
        line: usize,
        section: String,
        name: String,
    },
    #[error("line {line} holds no option name followed by = or :")]
    MalformedOption { line: usize },
}

#[derive(Debug)]
pub struct Section {
    pub name: String,
    pub fields: Vec<(String, SecretString)>,
}

#[derive(Debug, Default)]
pub struct IniFile {
    pub defaults: Vec<(String, SecretString)>,
    pub sections: Vec<Section>,
}

type Options = Vec<(String, Vec<String>)>;

enum Target {
    Nowhere,
    Defaults,
    Section(usize),
}

struct Reader {
    target: Target,
    option: Option<usize>,
    indent: usize,
    defaults: Options,
    sections: Vec<(String, Options)>,
}

// configparser strips with str.strip(), which also treats the four ASCII separators 0x1c to 0x1f as whitespace
const fn is_strip_space(character: char) -> bool {
    if matches!(character, '\u{1c}'..='\u{1f}') {
        return true;
    }
    character.is_whitespace()
}

// configparser's section pattern: a "[" first, then the text up to the last "]", which has to hold at least one character
fn header_name(clean: &str) -> Option<&str> {
    let inner = clean.strip_prefix('[')?;
    let end = inner.rfind(']').filter(|end| *end >= 1)?;
    inner.get(..end)
}

impl Reader {
    fn options_mut(&mut self) -> Option<&mut Options> {
        match self.target {
            Target::Nowhere => None,
            Target::Defaults => Some(&mut self.defaults),
            Target::Section(index) => self.sections.get_mut(index).map(|(_, options)| options),
        }
    }

    fn feed(&mut self, line: usize, raw: &str) -> Result<(), ParseError> {
        let clean = raw.trim_matches(is_strip_space);
        if clean.is_empty() || clean.starts_with(['#', ';']) {
            if clean.is_empty()
                && let Some(position) = self.option
                && let Some((_, lines)) = self
                    .options_mut()
                    .and_then(|options| options.get_mut(position))
            {
                lines.push(String::new());
            }
            return Ok(());
        }
        let indent = raw
            .chars()
            .take_while(|character| is_strip_space(*character))
            .count();
        if let Some(position) = self.option
            && indent > self.indent
        {
            if let Some((_, lines)) = self
                .options_mut()
                .and_then(|options| options.get_mut(position))
            {
                lines.push(clean.to_owned());
            }
            return Ok(());
        }
        self.indent = indent;
        match header_name(clean) {
            Some(name) => self.open_section(line, name),
            None => self.add_option(line, clean),
        }
    }

    fn open_section(&mut self, line: usize, name: &str) -> Result<(), ParseError> {
        self.option = None;
        if name == "DEFAULT" {
            self.target = Target::Defaults;
            return Ok(());
        }
        if self.sections.iter().any(|(known, _)| known == name) {
            return Err(ParseError::DuplicateSection {
                line,
                name: name.to_owned(),
            });
        }
        self.sections.push((name.to_owned(), Vec::new()));
        self.target = Target::Section(self.sections.len().saturating_sub(1));
        Ok(())
    }

    fn add_option(&mut self, line: usize, clean: &str) -> Result<(), ParseError> {
        if matches!(self.target, Target::Nowhere) {
            return Err(ParseError::MissingSectionHeader { line });
        }
        let (before, after) = clean
            .split_once(['=', ':'])
            .filter(|(before, _)| !before.is_empty())
            .ok_or(ParseError::MalformedOption { line })?;
        let name = before.trim_end_matches(is_strip_space).to_lowercase();
        let value = after.trim_matches(is_strip_space).to_owned();
        let section = match self.target {
            Target::Section(index) => self
                .sections
                .get(index)
                .map_or_else(String::new, |(section, _)| section.clone()),
            Target::Defaults | Target::Nowhere => "DEFAULT".to_owned(),
        };
        let Some(options) = self.options_mut() else {
            return Err(ParseError::MissingSectionHeader { line });
        };
        if options.iter().any(|(known, _)| *known == name) {
            return Err(ParseError::DuplicateOption {
                line,
                section,
                name,
            });
        }
        options.push((name, vec![value]));
        self.option = Some(options.len().saturating_sub(1));
        Ok(())
    }
}

fn joined(options: Options) -> Vec<(String, SecretString)> {
    options
        .into_iter()
        .map(|(name, lines)| {
            let value = lines.join("\n").trim_end_matches(is_strip_space).to_owned();
            (name, SecretString::from(value))
        })
        .collect()
}

// Parses the way configparser.ConfigParser(interpolation=None) does with its defaults; any error there rejects the whole file
pub fn parse(bytes: &[u8]) -> Result<IniFile, ParseError> {
    let text = str::from_utf8(bytes).map_err(ParseError::Utf8)?;
    let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
    let mut reader = Reader {
        target: Target::Nowhere,
        option: None,
        indent: 0,
        defaults: Vec::new(),
        sections: Vec::new(),
    };
    for (index, raw) in normalized.lines().enumerate() {
        reader.feed(index.saturating_add(1), raw)?;
    }
    Ok(IniFile {
        defaults: joined(reader.defaults),
        sections: reader
            .sections
            .into_iter()
            .map(|(name, options)| Section {
                name,
                fields: joined(options),
            })
            .collect(),
    })
}

impl IniFile {
    // ConfigParser.items(section): the DEFAULT entries first, a section's own value replacing a default of the same name
    #[must_use]
    pub fn with_defaults(self) -> Vec<Section> {
        let defaults = self.defaults;
        self.sections
            .into_iter()
            .map(|section| {
                let mut fields: Vec<(String, SecretString)> = defaults
                    .iter()
                    .map(|(name, value)| {
                        (
                            name.clone(),
                            SecretString::from(value.expose_secret().to_owned()),
                        )
                    })
                    .collect();
                for (name, value) in section.fields {
                    match fields.iter_mut().find(|(known, _)| *known == name) {
                        Some(slot) => slot.1 = value,
                        None => fields.push((name, value)),
                    }
                }
                Section {
                    name: section.name,
                    fields,
                }
            })
            .collect()
    }
}
