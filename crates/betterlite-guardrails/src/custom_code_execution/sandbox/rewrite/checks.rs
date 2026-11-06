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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum NameKind {
    Variable,
    Attribute,
}

pub(super) fn check_name(
    name: &str,
    filename: &str,
    line: u32,
    kind: NameKind,
) -> Result<(), CompileError> {
    let subject = match kind {
        NameKind::Variable => "variable",
        NameKind::Attribute => "attribute",
    };
    if name.starts_with('_') && name != "_" {
        return Err(CompileError::new(
            filename,
            line,
            format!("\"{name}\" is an invalid {subject} name because it starts with \"_\""),
        ));
    }
    if name.ends_with("__roles__") {
        return Err(CompileError::new(
            filename,
            line,
            format!("\"{name}\" is an invalid {subject} name because it ends with \"__roles__\""),
        ));
    }
    match kind {
        NameKind::Variable if FORBIDDEN_NAMES.contains(&name) => Err(CompileError::new(
            filename,
            line,
            format!("\"{name}\" is a reserved name."),
        )),
        NameKind::Attribute if INSPECT_ATTRIBUTES.contains(&name) => Err(CompileError::new(
            filename,
            line,
            format!(
                "\"{name}\" is a restricted name, that is forbidden to access in RestrictedPython."
            ),
        )),
        NameKind::Variable | NameKind::Attribute => Ok(()),
    }
}

pub(super) fn check_call(name: &str, filename: &str, line: u32) -> Result<(), CompileError> {
    match name {
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
