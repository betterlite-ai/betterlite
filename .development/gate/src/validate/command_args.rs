const AFTER_SEPARATOR: [&str; 5] = [
    "--exact",
    "--skip",
    "--ignored",
    "--include-ignored",
    "--nocapture",
];
const LIBTEST_ONLY: [&str; 4] = ["--exact", "--skip", "--ignored", "--include-ignored"];

struct ShellWord {
    text: String,
    option: String,
    quoted: bool,
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

fn nextest_problems(words: &[ShellWord]) -> Vec<String> {
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
    let (before, after) = arguments.split_at_checked(separator).unwrap_or_default();
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
    early.chain(late).collect()
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

pub(crate) fn argument_problems(run: &str) -> Vec<String> {
    let mut problems = Vec::new();
    let letters = quoted_letters(run);
    for command in letters.split(|&(letter, quoted)| !quoted && ";|&()`\n".contains(letter)) {
        let words: Vec<ShellWord> = command
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
            .collect();
        problems.extend(nextest_problems(&words));
        problems.extend(grep_problem(&words));
    }
    problems
}
