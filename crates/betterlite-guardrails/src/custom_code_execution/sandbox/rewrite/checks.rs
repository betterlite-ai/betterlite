use super::super::error::CompileError;
const RESERVED_NAMES: &str = " print printed builtins breakpoint";
const ALLOWED_METHODS: &str = " __init__ __contains__ __lt__ __le__ __eq__ __ne__ __gt__ __ge__";
const INSPECT_ATTRIBUTES: &str = " tb_frame tb_next f_back f_builtins f_code f_generator f_globals f_locals f_trace co_code gi_frame gi_code gi_yieldfrom cr_await cr_frame cr_code cr_origin ag_await ag_frame ag_code";
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum NameKind {
    Variable,
    Method,
    Attribute,
}
pub(super) fn check_name(
    name: &str,
    filename: &str,
    line: u32,
    kind: NameKind,
) -> Result<(), CompileError> {
    let subject = match kind {
        NameKind::Variable | NameKind::Method => "variable",
        NameKind::Attribute => "attribute",
    };
    let magic = kind == NameKind::Method
        && ALLOWED_METHODS
            .split_whitespace()
            .any(|candidate| candidate == name);
    let problem = if !magic && name.starts_with('_') && name != "_" {
        Some(format!(
            "\"{name}\" is an invalid {subject} name because it starts with \"_\""
        ))
    } else if name.ends_with("__roles__") {
        Some(format!(
            "\"{name}\" is an invalid {subject} name because it ends with \"__roles__\""
        ))
    } else if kind != NameKind::Attribute
        && RESERVED_NAMES
            .split_whitespace()
            .any(|candidate| candidate == name)
    {
        Some(format!("\"{name}\" is a reserved name."))
    } else if kind == NameKind::Attribute
        && INSPECT_ATTRIBUTES
            .split_whitespace()
            .any(|candidate| candidate == name)
    {
        Some(format!(
            "\"{name}\" is a restricted name, that is forbidden to access in RestrictedPython."
        ))
    } else {
        None
    };
    problem.map_or(Ok(()), |message| {
        Err(CompileError::forbidden(filename, line, message))
    })
}
pub(super) fn check_call(name: &str, filename: &str, line: u32) -> Result<(), CompileError> {
    match name {
        "exec" => Err(CompileError::forbidden(
            filename,
            line,
            "Exec calls are not allowed.".to_owned(),
        )),
        "eval" => Err(CompileError::forbidden(
            filename,
            line,
            "Eval calls are not allowed.".to_owned(),
        )),
        _ => Ok(()),
    }
}
