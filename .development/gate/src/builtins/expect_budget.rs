pub(crate) fn check(
    limit: usize,
    pin: &str,
    rust_lines: &[String],
    python_lines: &[String],
    caller_lines: &[String],
) -> Vec<String> {
    let found: Vec<&String> = rust_lines
        .iter()
        .filter(|line| line.contains("#[expect(") || line.contains("#![expect("))
        .chain(python_lines.iter().filter(|line| suppresses(line)))
        // a suppression LiteLLM forces in the oracle code that calls it, shown by a link at the pinned commit, is free (Max, 11-15 helpers, 11-19 sdk_http)
        .chain(
            caller_lines
                .iter()
                .filter(|line| suppresses(line) && !line.contains(pin)),
        )
        .collect();
    if found.len() <= limit {
        return Vec::new();
    }
    let mut problems = vec![format!(
        "{} new #[expect(...)] attributes and Python lines with a cast, type: ignore, pyright: ignore or noqa, budget is {limit}:",
        found.len()
    )];
    problems.extend(found.iter().map(|line| line.trim().to_owned()));
    problems
}

fn suppresses(line: &str) -> bool {
    let casts = line.match_indices("cast(").any(|(at, _)| {
        line.get(..at)
            .and_then(|before| before.chars().next_back())
            .is_none_or(|previous| !previous.is_alphanumeric() && previous != '_')
    });
    casts
        || line.split('#').skip(1).any(|after_hash| {
            let comment = after_hash.trim_start();
            comment.starts_with("noqa") || ignores(comment, "type:") || ignores(comment, "pyright:")
        })
}

fn ignores(comment: &str, tool: &str) -> bool {
    comment
        .strip_prefix(tool)
        .is_some_and(|rest| rest.trim_start().starts_with("ignore"))
}
