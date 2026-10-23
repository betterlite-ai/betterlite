pub(crate) fn task_id_problem(text: &str) -> Option<String> {
    let digits = text.strip_prefix('B');
    let valid = digits
        .is_some_and(|rest| rest.len() == 3 && rest.bytes().all(|byte| byte.is_ascii_digit()));
    (!valid).then(|| format!("{text} is not a task id like B004"))
}

pub(crate) fn check_id_problem(text: &str) -> Option<String> {
    let starts_with_letter = text
        .bytes()
        .next()
        .is_some_and(|byte| byte.is_ascii_lowercase());
    let kebab = text
        .bytes()
        .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-');
    (!(starts_with_letter && kebab)).then(|| format!("{text} is not a lowercase kebab-case id"))
}
