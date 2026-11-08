use std::ops::Range;

use crate::tools::error::ToolFormatError;
use crate::tools::json::{self, Field, Scanner};
use crate::tools::schema::{self, SchemaNode};

#[derive(Debug)]
pub(super) struct Tool {
    function: Option<Function>,
    declarations: Option<Vec<Option<Function>>>,
    camel_declarations: Option<Vec<Option<Function>>>,
    inline: Function,
}

#[derive(Debug)]
struct Function {
    name: Option<Field<Range<usize>>>,
    description: Option<Field<Range<usize>>>,
    parameters: Option<Field<Box<SchemaNode>>>,
    parameter_field: &'static str,
}

impl Default for Function {
    fn default() -> Self {
        Self {
            name: None,
            description: None,
            parameters: None,
            parameter_field: "parameters",
        }
    }
}

#[derive(Default)]
struct ToolParts {
    function: Option<Function>,
    declarations: Option<Vec<Option<Function>>>,
    camel_declarations: Option<Vec<Option<Function>>>,
    input_schema: Option<Field<Box<SchemaNode>>>,
    parameters: Option<Field<Box<SchemaNode>>>,
    name: Option<Field<Range<usize>>>,
    description: Option<Field<Range<usize>>>,
}

impl ToolParts {
    fn resolve(self) -> Tool {
        let from_schema = self.input_schema.is_some();
        let parameters = self.input_schema.or(self.parameters);
        Tool {
            function: self.function,
            declarations: self.declarations,
            camel_declarations: self.camel_declarations,
            inline: Function {
                name: self.name,
                description: self.description,
                parameters,
                parameter_field: if from_schema {
                    "input_schema"
                } else {
                    "parameters"
                },
            },
        }
    }
}

pub(super) struct Formatter<'json> {
    source: &'json [u8],
    limit: usize,
    used: usize,
    pub(super) lines: Vec<String>,
}

impl<'json> Formatter<'json> {
    pub(super) const fn new(source: &'json [u8], limit: usize, lines: Vec<String>) -> Self {
        Self {
            source,
            limit,
            used: 0,
            lines,
        }
    }

    pub(super) fn push_tool(
        &mut self,
        index: usize,
        tool: Option<&Tool>,
    ) -> Result<(), ToolFormatError> {
        let Some(selected) = tool else {
            return Ok(());
        };
        self.tool(index, selected)
    }

    fn tool(&mut self, index: usize, tool: &Tool) -> Result<(), ToolFormatError> {
        if let Some(function) = &tool.function {
            self.used = charge(self.used, 1, self.limit)?;
            let formatted = self.function(index, function)?;
            self.lines.extend(formatted);
            return Ok(());
        }
        let declarations = tool
            .declarations
            .as_ref()
            .or(tool.camel_declarations.as_ref());
        if let Some(entries) = declarations {
            self.used = charge(self.used, entries.len(), self.limit)?;
            for function in entries.iter().flatten() {
                let formatted = self.function(index, function)?;
                self.lines.extend(formatted);
            }
            return Ok(());
        }
        self.used = charge(self.used, 1, self.limit)?;
        let formatted = self.function(index, &tool.inline)?;
        self.lines.extend(formatted);
        Ok(())
    }

    fn function(&self, index: usize, function: &Function) -> Result<Vec<String>, ToolFormatError> {
        let name = text_field(self.source, index, function.name.as_ref(), "name")?;
        let description = text_field(
            self.source,
            index,
            function.description.as_ref(),
            "description",
        )?;
        let parameters = function
            .parameters
            .as_ref()
            .map(|field| field.value(function.parameter_field, "a JSON object"))
            .transpose()?
            .map(Box::as_ref);
        format_definition(self.source, name, description, parameters)
    }
}

fn text_field(
    source: &[u8],
    index: usize,
    value: Option<&Field<Range<usize>>>,
    name: &'static str,
) -> Result<Option<String>, ToolFormatError> {
    let Some(field) = value else {
        return Ok(None);
    };
    let range = field.shape(index, name)?;
    json::text(source, range, name).map(Some)
}

const fn charge(used: usize, count: usize, limit: usize) -> Result<usize, ToolFormatError> {
    let total = used.saturating_add(count);
    if total > limit {
        return Err(ToolFormatError::TooMany {
            count: total,
            limit,
        });
    }
    Ok(total)
}

fn format_definition(
    source: &[u8],
    function_name: Option<String>,
    description: Option<String>,
    parameters: Option<&SchemaNode>,
) -> Result<Vec<String>, ToolFormatError> {
    // a function with no name has no type declaration to emit, as in LiteLLM
    let Some(name) = function_name.filter(|candidate| !candidate.is_empty()) else {
        return Ok(Vec::new());
    };
    let mut lines = Vec::new();
    if let Some(text) = description {
        lines.push(format!("// {text}"));
    }
    let Some(parameter_object) = parameters else {
        return Ok(single_parameter_list(lines, &name));
    };
    let rendered = schema::object_parameters(source, parameter_object, 0, 0)?;
    if rendered.is_empty() {
        return Ok(single_parameter_list(lines, &name));
    }
    lines.push(format!("type {name} = (_: {{"));
    lines.push(rendered);
    lines.push("}) => any;".to_owned());
    lines.push(String::new());
    Ok(lines)
}

fn single_parameter_list(mut lines: Vec<String>, name: &str) -> Vec<String> {
    lines.push(format!("type {name} = () => any;"));
    lines.push(String::new());
    lines
}

pub(super) fn parse_tool(scanner: &mut Scanner<'_>) -> Result<Option<Tool>, ToolFormatError> {
    if scanner.peek() != Some(b'{') {
        scanner.skip_value()?;
        return Ok(None);
    }
    let parts = json::object_entries(scanner, ToolParts::default(), |parts, reader, key| {
        assign(Member::Tool(parts), reader, &key)
    })?;
    Ok(Some(parts.resolve()))
}

enum Member<'member> {
    Tool(&'member mut ToolParts),
    Function(&'member mut Function),
}

fn assign(member: Member<'_>, scanner: &mut Scanner<'_>, key: &str) -> Result<(), ToolFormatError> {
    match (member, key) {
        (Member::Tool(parts), "function") => parts.function = parse_function_member(scanner)?,
        (Member::Tool(parts), "function_declarations") => {
            parts.declarations = parse_declarations(scanner)?;
        }
        (Member::Tool(parts), "functionDeclarations") => {
            parts.camel_declarations = parse_declarations(scanner)?;
        }
        (Member::Tool(parts), "input_schema") => {
            parts.input_schema =
                json::keyword(scanner, b'{', |reader| schema::parse_node(reader, 0))?;
        }
        (Member::Tool(parts), "parameters") => {
            parts.parameters =
                json::keyword(scanner, b'{', |reader| schema::parse_node(reader, 0))?;
        }
        (Member::Tool(parts), "name") => {
            parts.name = json::keyword(scanner, b'"', Scanner::skip_value)?;
        }
        (Member::Tool(parts), "description") => {
            parts.description = json::keyword(scanner, b'"', Scanner::skip_value)?;
        }
        (Member::Function(function), "parameters") => {
            function.parameters =
                json::keyword(scanner, b'{', |reader| schema::parse_node(reader, 0))?;
        }
        (Member::Function(function), "name") => {
            function.name = json::keyword(scanner, b'"', Scanner::skip_value)?;
        }
        (Member::Function(function), "description") => {
            function.description = json::keyword(scanner, b'"', Scanner::skip_value)?;
        }
        (Member::Tool(_) | Member::Function(_), _) => {
            scanner.skip_value()?;
        }
    }
    Ok(())
}

fn parse_function_member(scanner: &mut Scanner<'_>) -> Result<Option<Function>, ToolFormatError> {
    if scanner.peek() != Some(b'{') {
        scanner.skip_value()?;
        return Ok(None);
    }
    parse_function(scanner).map(Some)
}

fn parse_function(scanner: &mut Scanner<'_>) -> Result<Function, ToolFormatError> {
    json::object_entries(scanner, Function::default(), |function, reader, key| {
        assign(Member::Function(function), reader, &key)
    })
}

fn parse_declarations(
    scanner: &mut Scanner<'_>,
) -> Result<Option<Vec<Option<Function>>>, ToolFormatError> {
    if scanner.peek() != Some(b'[') {
        scanner.skip_value()?;
        return Ok(None);
    }
    let mut entries = Vec::new();
    scanner.take_byte(b'[')?;
    if scanner.peek() == Some(b']') {
        scanner.take_byte(b']')?;
        return Ok(Some(entries));
    }
    loop {
        if scanner.peek() == Some(b'{') {
            entries.push(parse_function(scanner).map(Some)?);
        } else {
            scanner.skip_value()?;
            entries.push(None);
        }
        match scanner.peek() {
            Some(b',') => scanner.take_byte(b',')?,
            Some(b']') => {
                scanner.take_byte(b']')?;
                return Ok(Some(entries));
            }
            Some(_) | None => return Err(json::broken()),
        }
    }
}
