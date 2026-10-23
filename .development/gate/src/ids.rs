pub(crate) fn task_id_problem(text: &str) -> Option<String> {
    let digits = text.strip_prefix('B').unwrap_or_default();
    let numeric = !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit());
    // Only three-digit ids keep leading zeros, so every number has exactly one spelling.
    let canonical =
        digits.len() == 3 || ((4..=6).contains(&digits.len()) && !digits.starts_with('0'));
    (!(numeric && canonical)).then(|| format!("{text} is not a task id like B004 or B1234"))
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
