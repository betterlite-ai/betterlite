use std::collections::BTreeSet;

use nextest_filtering::{ParsedExpr, errors::ParseSingleError};

const AFTER_SEPARATOR: [&str; 5] = [
    "--exact",
    "--skip",
    "--ignored",
    "--include-ignored",
    "--nocapture",
];
const LIBTEST_ONLY: [&str; 4] = ["--exact", "--skip", "--ignored", "--include-ignored"];
const FILTERSET_OPTIONS: [&str; 3] = ["-E", "--filterset", "--filter-expr"];
const PACKAGE_OPTIONS: [&str; 2] = ["-p", "--package"];
const TARGET_OPTIONS: [&str; 8] = [
    "--test",
    "--tests",
    "--bin",
    "--bins",
    "--example",
    "--examples",
    "--bench",
    "--benches",
];
const BINARY_FILTERS: [&str; 3] = ["binary(", "binary_id(", "kind("];

struct ShellWord {
    text: String,
    option: String,
    quoted: bool,
}

pub(crate) struct FilteredRun {
    pub(crate) packages: Vec<String>,
    pub(crate) modules: BTreeSet<String>,
}

// Reads quoting only; variables, globs and $(...) stay as written
fn quoted_letters(run: &str) -> Vec<(char, bool)> {
    let mut letters = Vec::new();
    let mut quote = None;
    let mut chars = run.chars();
    while let Some(letter) = chars.next() {
        match (quote, letter) {
            (Some(open), _) if open == letter => quote = None,
            (Some(_), _) => letters.push((letter, true)),
            (None, '\'' | '"') => quote = Some(letter),
            (None, '\\') => letters.extend(
                chars
                    .next()
                    .filter(|next| *next != '\n')
                    .map(|next| (next, true)),
            ),
            (None, _) => letters.push((letter, false)),
        }
    }
    letters
}

fn option_values<'words>(before: &'words [ShellWord], options: &[&str]) -> Vec<&'words str> {
    let mut values = Vec::new();
    for pair in before.windows(2) {
        if let [flag, value] = pair
            && options.contains(&flag.text.as_str())
        {
            values.push(value.text.as_str());
        }
    }
    for word in before {
        if let Some((option, value)) = word.text.split_once('=')
            && options.contains(&option)
        {
            values.push(value);
        }
    }
    values
}

fn filterset_problems(before: &[ShellWord]) -> Vec<String> {
    let mut problems = Vec::new();
    for filterset in option_values(before, &FILTERSET_OPTIONS) {
        for error in ParsedExpr::parse(filterset).err().unwrap_or_default() {
            let detail = if let ParseSingleError::InvalidRegex { message, .. } = &error {
                format!("{error}: {message}")
            } else {
                format!("{error}")
            };
            problems.push(format!(
                "passes the filterset {filterset} that cargo nextest can't parse ({detail})"
            ));
        }
    }
    problems
}

fn nextest_arguments(words: &[ShellWord]) -> (&[ShellWord], &[ShellWord]) {
    let arguments = words
        .windows(3)
        .position(|three| {
            three
                .iter()
                .map(|word| word.text.as_str())
                .eq(["cargo", "nextest", "run"])
        })
        .and_then(|start| words.get(start.checked_add(3)?..))
        .unwrap_or_default();
    let separator = arguments
        .iter()
        .position(|word| word.text == "--")
        .unwrap_or(arguments.len());
    arguments.split_at_checked(separator).unwrap_or_default()
}

fn nextest_problems(words: &[ShellWord]) -> Vec<String> {
    let (before, after) = nextest_arguments(words);
    let early = before
        .iter()
        .filter(|word| LIBTEST_ONLY.contains(&word.option.as_str()))
        .map(|word| {
            format!(
                "puts {} before --; cargo nextest run takes it only after --",
                word.text
            )
        });
    let late = after
        .iter()
        .skip(1)
        .filter(|word| {
            word.text.starts_with('-') && !AFTER_SEPARATOR.contains(&word.option.as_str())
        })
        .map(|word| {
            format!(
                "passes {} after --; cargo nextest run takes only test name filters and {} there",
                word.text,
                AFTER_SEPARATOR.join(", ")
            )
        });
    let mut problems: Vec<String> = early.chain(late).collect();
    problems.extend(filterset_problems(before));
    problems
}

fn grep_problem(words: &[ShellWord]) -> Option<String> {
    for word in words.iter().skip_while(|word| word.text != "grep").skip(1) {
        if word.quoted && word.text.starts_with('-') {
            return Some(format!(
                "gives grep the pattern {} before --, so grep reads it as an option; put -- before the pattern",
                word.text
            ));
        }
        let short = word.text.starts_with('-') && !word.text.starts_with("--");
        let pattern_next = (short && (word.text.ends_with('e') || word.text.ends_with('f')))
            || ["--", "--regexp", "--file"].contains(&word.option.as_str());
        if pattern_next || !word.text.starts_with('-') {
            return None;
        }
    }
    None
}

fn shell_commands(run: &str) -> Vec<Vec<ShellWord>> {
    let letters = quoted_letters(run);
    letters
        .split(|&(letter, quoted)| !quoted && ";|&()`\n".contains(letter))
        .map(|command| {
            command
                .split(|&(letter, quoted)| {
                    !quoted && (letter.is_whitespace() || letter == '<' || letter == '>')
                })
                .filter_map(|word| {
                    let &(_, quoted) = word.first()?;
                    let text: String = word.iter().map(|&(letter, _)| letter).collect();
                    let option = text
                        .split_once('=')
                        .map_or(text.as_str(), |(name, _)| name)
                        .to_owned();
                    Some(ShellWord {
                        text,
                        option,
                        quoted,
                    })
                })
                .collect()
        })
        .collect()
}

pub(crate) fn argument_problems(run: &str) -> Vec<String> {
    let mut problems = Vec::new();
    for words in shell_commands(run) {
        problems.extend(nextest_problems(&words));
        problems.extend(grep_problem(&words));
    }
    problems
}

// A run that picks test binaries may name modules of an integration test, not of src
pub(crate) fn filtered_runs(run: &str) -> Vec<FilteredRun> {
    let mut runs = Vec::new();
    for words in shell_commands(run) {
        let (before, _) = nextest_arguments(&words);
        let filtersets = option_values(before, &FILTERSET_OPTIONS);
        let targeted = before
            .iter()
            .any(|word| TARGET_OPTIONS.contains(&word.option.as_str()))
            || filtersets
                .iter()
                .any(|filterset| BINARY_FILTERS.iter().any(|leaf| filterset.contains(leaf)));
        if targeted {
            continue;
        }
        let mut modules = BTreeSet::new();
        for argument in filtersets
            .iter()
            .flat_map(|filterset| filterset.split("test(").skip(1))
        {
            let anchored = argument
                .strip_prefix("/^")
                .or_else(|| argument.strip_prefix('='));
            if let Some((module, _)) = anchored.and_then(|path| path.split_once("::"))
                && !module.is_empty()
                && module.chars().all(|letter| {
                    letter.is_ascii_lowercase() || letter.is_ascii_digit() || letter == '_'
                })
            {
                modules.insert(module.to_owned());
            }
        }
        let packages: Vec<String> = option_values(before, &PACKAGE_OPTIONS)
            .into_iter()
            .map(str::to_owned)
            .collect();
        if !modules.is_empty() && !packages.is_empty() {
            runs.push(FilteredRun { packages, modules });
        }
    }
    runs
}
