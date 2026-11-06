use tree_sitter::Node;

use super::super::error::CompileError;

const FORBIDDEN_NAMES: [&str; 4] = ["print", "printed", "builtins", "breakpoint"];

const INSPECT_ATTRIBUTES: [&str; 20] = [
    "tb_frame",
    "tb_next",
    "f_back",
    "f_builtins",
    "f_code",
    "f_generator",
    "f_globals",
    "f_locals",
    "f_trace",
    "co_code",
    "gi_frame",
    "gi_code",
    "gi_yieldfrom",
    "cr_await",
    "cr_frame",
    "cr_code",
    "cr_origin",
    "ag_await",
    "ag_frame",
    "ag_code",
];

pub(super) fn check_name(name: &str, filename: &str, line: u32) -> Result<(), CompileError> {
    if name.starts_with('_') && name != "_" {
        return Err(CompileError::new(
            filename,
            line,
            format!("\"{name}\" is an invalid variable name because it starts with \"_\""),
        ));
    }
    if name.ends_with("__roles__") {
        return Err(CompileError::new(
            filename,
            line,
            format!("\"{name}\" is an invalid variable name because it ends with \"__roles__\""),
        ));
    }
    if FORBIDDEN_NAMES.contains(&name) {
        return Err(CompileError::new(
            filename,
            line,
            format!("\"{name}\" is a reserved name."),
        ));
    }
    Ok(())
}

fn check_attribute(attribute: &str, filename: &str, line: u32) -> Result<(), CompileError> {
    if attribute.starts_with('_') && attribute != "_" {
        return Err(CompileError::new(
            filename,
            line,
            format!("\"{attribute}\" is an invalid attribute name because it starts with \"_\""),
        ));
    }
    if attribute.ends_with("__roles__") {
        return Err(CompileError::new(
            filename,
            line,
            format!(
                "\"{attribute}\" is an invalid attribute name because it ends with \"__roles__\""
            ),
        ));
    }
    if INSPECT_ATTRIBUTES.contains(&attribute) {
        return Err(CompileError::new(
            filename,
            line,
            format!(
                "\"{attribute}\" is a restricted name, that is forbidden to access in RestrictedPython."
            ),
        ));
    }
    Ok(())
}

pub(super) fn check_attribute_node(
    node: Node<'_>,
    source: &str,
    filename: &str,
    line: u32,
) -> Result<(), CompileError> {
    let Some(attribute) = node.child_by_field_name("attribute") else {
        return Ok(());
    };
    check_attribute(
        source.get(attribute.byte_range()).unwrap_or_default(),
        filename,
        line,
    )
}

pub(super) fn check_call_node(
    node: Node<'_>,
    source: &str,
    filename: &str,
    line: u32,
) -> Result<(), CompileError> {
    let Some(function) = node.child_by_field_name("function") else {
        return Ok(());
    };
    if function.kind() != "identifier" {
        return Ok(());
    }
    match source.get(function.byte_range()).unwrap_or_default() {
        "exec" => Err(CompileError::new(
            filename,
            line,
            "Exec calls are not allowed.".to_owned(),
        )),
        "eval" => Err(CompileError::new(
            filename,
            line,
            "Eval calls are not allowed.".to_owned(),
        )),
        _ => Ok(()),
    }
}

// An attribute's own name is checked by the attribute rule, which reports it as an attribute.
pub(super) fn check_identifier(
    node: Node<'_>,
    source: &str,
    filename: &str,
    line: u32,
) -> Result<(), CompileError> {
    if node
        .parent()
        .is_some_and(|parent| parent.kind() == "attribute")
    {
        return Ok(());
    }
    check_name(
        source.get(node.byte_range()).unwrap_or_default(),
        filename,
        line,
    )
}
