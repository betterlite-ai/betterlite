use std::ops::Range;

use crate::tools::error::ToolFormatError;
use crate::tools::json::{self, Field, Scanner};
use crate::tools::schema::{self, SchemaNode};

#[derive(Debug, Default)]
pub(super) struct Tool {
    function: Option<Function>,
    declarations: Option<Vec<Option<Function>>>,
    camel_declarations: Option<Vec<Option<Function>>>,
    input_schema: Option<Field<Box<SchemaNode>>>,
    parameters: Option<Field<Box<SchemaNode>>>,
    name: Option<Field<Range<usize>>>,
    description: Option<Field<Range<usize>>>,
}

#[derive(Debug, Default)]
struct Function {
    name: Option<Field<Range<usize>>>,
    description: Option<Field<Range<usize>>>,
    parameters: Option<Field<Box<SchemaNode>>>,
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
        let formatted = self.inline(index, tool)?;
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
            .map(|field| field.value("parameters", "a JSON object"))
            .transpose()?
            .map(Box::as_ref);
        format_definition(self.source, name, description, parameters)
    }

    fn inline(&self, index: usize, tool: &Tool) -> Result<Vec<String>, ToolFormatError> {
        let (field, selected) = tool.input_schema.as_ref().map_or_else(
            || ("parameters", tool.parameters.as_ref()),
            |schema| ("input_schema", Some(schema)),
        );
        let parameters = selected
            .map(|value| value.value(field, "a JSON object"))
            .transpose()?
            .map(Box::as_ref);
        let name = text_field(self.source, index, tool.name.as_ref(), "name")?;
        let description = text_field(self.source, index, tool.description.as_ref(), "description")?;
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
    let mut tool = Tool::default();
    scanner.take_byte(b'{')?;
    if scanner.peek() == Some(b'}') {
        scanner.take_byte(b'}')?;
        return Ok(Some(tool));
    }
    loop {
        if scanner.peek() != Some(b'"') {
            return Err(json::broken());
        }
        let key = scanner.parse_string("function")?;
        scanner.take_byte(b':')?;
        assign(&mut tool, scanner, &key)?;
        match scanner.peek() {
            Some(b',') => scanner.take_byte(b',')?,
            Some(b'}') => {
                scanner.take_byte(b'}')?;
                return Ok(Some(tool));
            }
            Some(_) | None => return Err(json::broken()),
        }
    }
}

fn assign(tool: &mut Tool, scanner: &mut Scanner<'_>, key: &str) -> Result<(), ToolFormatError> {
    match key {
        "function" => tool.function = parse_function_member(scanner)?,
        "function_declarations" => tool.declarations = parse_declarations(scanner)?,
        "functionDeclarations" => tool.camel_declarations = parse_declarations(scanner)?,
        "input_schema" => tool.input_schema = parse_schema_member(scanner)?,
        "parameters" => tool.parameters = parse_schema_member(scanner)?,
        "name" => tool.name = parse_text_member(scanner)?,
        "description" => tool.description = parse_text_member(scanner)?,
        _ => {
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
    let mut function = Function::default();
    scanner.take_byte(b'{')?;
    if scanner.peek() == Some(b'}') {
        scanner.take_byte(b'}')?;
        return Ok(function);
    }
    loop {
        if scanner.peek() != Some(b'"') {
            return Err(json::broken());
        }
        let key = scanner.parse_string("function")?;
        scanner.take_byte(b':')?;
        match key.as_str() {
            "name" => function.name = parse_text_member(scanner)?,
            "description" => function.description = parse_text_member(scanner)?,
            "parameters" => function.parameters = parse_schema_member(scanner)?,
            _ => {
                scanner.skip_value()?;
            }
        }
        match scanner.peek() {
            Some(b',') => scanner.take_byte(b',')?,
            Some(b'}') => {
                scanner.take_byte(b'}')?;
                return Ok(function);
            }
            Some(_) | None => return Err(json::broken()),
        }
    }
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

fn parse_schema_member(
    scanner: &mut Scanner<'_>,
) -> Result<Option<Field<Box<SchemaNode>>>, ToolFormatError> {
    match scanner.peek() {
        Some(b'{') => {
            let node = schema::parse_node(scanner, 0)?;
            Ok(Some(Field::Value(Box::new(node))))
        }
        Some(b'n') => {
            scanner.skip_value()?;
            Ok(None)
        }
        Some(byte) => {
            scanner.skip_value()?;
            Ok(Some(Field::Wrong(json::kind_name(byte))))
        }
        None => Err(json::broken()),
    }
}

fn parse_text_member(
    scanner: &mut Scanner<'_>,
) -> Result<Option<Field<Range<usize>>>, ToolFormatError> {
    match scanner.peek() {
        Some(b'"') => Ok(Some(Field::Value(scanner.skip_value()?))),
        Some(b'n') => {
            scanner.skip_value()?;
            Ok(None)
        }
        Some(byte) => {
            scanner.skip_value()?;
            Ok(Some(Field::Wrong(json::kind_name(byte))))
        }
        None => Err(json::broken()),
    }
}
