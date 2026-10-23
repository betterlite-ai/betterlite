pub(crate) fn check(limit: usize, added_lines: &[String]) -> Vec<String> {
    let found: Vec<&String> = added_lines
        .iter()
        .filter(|line| line.contains("#[expect(") || line.contains("#![expect("))
        .collect();
    if found.len() <= limit {
        return Vec::new();
    }
    let mut problems = vec![format!(
        "{} new #[expect(...)] attributes, budget is {limit}:",
        found.len()
    )];
    problems.extend(found.iter().map(|line| line.trim().to_owned()));
    problems
}
