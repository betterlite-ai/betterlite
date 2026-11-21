use std::ops::RangeInclusive;

#[derive(Debug, Clone, PartialEq, Eq)]
enum Token {
    Literal(char),
    AnyOne,
    AnyRun,
    Set {
        negated: bool,
        ranges: Vec<RangeInclusive<char>>,
    },
}

impl Token {
    fn accepts(&self, letter: char) -> bool {
        match self {
            Self::Literal(expected) => *expected == letter,
            Self::AnyOne => true,
            Self::AnyRun => false,
            Self::Set { negated, ranges } => {
                ranges.iter().any(|range| range.contains(&letter)) != *negated
            }
        }
    }
}

// Python 3.13's fnmatch on Linux: case-sensitive, '/' is ordinary and an unclosed '[' is a literal
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RolePattern {
    tokens: Vec<Token>,
}

impl RolePattern {
    #[must_use]
    pub fn new(pattern: &str) -> Self {
        let letters = pattern.chars().collect::<Vec<char>>();
        let mut tokens = Vec::new();
        let mut rest = letters.as_slice();
        while let Some((letter, tail)) = rest.split_first() {
            rest = tail;
            match letter {
                '*' if tokens.last() == Some(&Token::AnyRun) => {}
                '*' => tokens.push(Token::AnyRun),
                '?' => tokens.push(Token::AnyOne),
                '[' => {
                    let Some((set, after)) = bracket(tail) else {
                        tokens.push(Token::Literal('['));
                        continue;
                    };
                    tokens.push(set);
                    rest = after;
                }
                other => tokens.push(Token::Literal(*other)),
            }
        }
        Self { tokens }
    }

    // every token but a run takes one letter, so resuming after the last run is enough
    #[must_use]
    pub fn matches(&self, name: &str) -> bool {
        let letters = name.chars().collect::<Vec<char>>();
        let mut pattern = self.tokens.as_slice();
        let mut text = letters.as_slice();
        let mut resume = None;
        loop {
            if let Some((Token::AnyRun, after)) = pattern.split_first() {
                resume = Some((after, text));
                pattern = after;
                continue;
            }
            if let (Some((token, after)), Some((letter, tail))) =
                (pattern.split_first(), text.split_first())
                && token.accepts(*letter)
            {
                pattern = after;
                text = tail;
                continue;
            }
            if pattern.is_empty() && text.is_empty() {
                return true;
            }
            let Some((after_run, swallowed)) = resume else {
                return false;
            };
            let Some((_, tail)) = swallowed.split_first() else {
                return false;
            };
            resume = Some((after_run, tail));
            pattern = after_run;
            text = tail;
        }
    }
}

// the text after '[': the set and what follows its ']', or None when no ']' closes it
fn bracket(tail: &[char]) -> Option<(Token, &[char])> {
    let opening = match tail {
        ['!', ']', ..] => 2,
        ['!', ..] | [']', ..] => 1,
        _ => 0,
    };
    let (head, scan) = tail.split_at_checked(opening)?;
    let close = scan.iter().position(|letter| *letter == ']')?;
    let (body, closing) = scan.split_at_checked(close)?;
    let after = closing.get(1..)?;
    let content = [head, body].concat();
    Some((set_token(&content), after))
}

// fnmatch._translate: '-' splits chunks into ranges, and a reversed range folds into its neighbours
fn set_token(content: &[char]) -> Token {
    let mut chunks = Vec::new();
    let mut rest = content;
    let mut guarded = if content.first() == Some(&'!') { 2 } else { 1 };
    while let Some((head, scan)) = rest.split_at_checked(guarded)
        && let Some(hyphen) = scan.iter().position(|letter| *letter == '-')
        && let Some((body, from_hyphen)) = scan.split_at_checked(hyphen)
    {
        chunks.push([head, body].concat());
        rest = from_hyphen.get(1..).unwrap_or_default();
        guarded = 2;
    }
    if rest.is_empty() {
        if let Some(last) = chunks.last_mut() {
            last.push('-');
        }
    } else {
        chunks.push(rest.to_vec());
    }
    let mut merged = fold_reversed(chunks);
    // translate() reads '!' after the fold, so a fold that brings one to the front negates the set
    let negated = merged.first().and_then(|first| first.first()) == Some(&'!');
    if negated && let Some(first) = merged.first_mut() {
        let kept = first.get(1..).unwrap_or_default().to_vec();
        *first = kept;
    }
    Token::Set {
        negated,
        ranges: ranges(&merged),
    }
}

fn fold_reversed(chunks: Vec<Vec<char>>) -> Vec<Vec<char>> {
    let mut folded: Vec<Vec<char>> = Vec::new();
    for chunk in chunks.into_iter().rev() {
        let Some(next) = folded.pop() else {
            folded.push(chunk);
            continue;
        };
        if chunk.last() > next.first() {
            let kept = chunk.split_last().map_or(&[][..], |(_, head)| head);
            let joined = next.split_first().map_or(&[][..], |(_, tail)| tail);
            folded.push([kept, joined].concat());
        } else {
            folded.push(next);
            folded.push(chunk);
        }
    }
    folded.reverse();
    folded
}

// inside a chunk every letter stands for itself; between chunks the touching letters bound a range
fn ranges(chunks: &[Vec<char>]) -> Vec<RangeInclusive<char>> {
    let mut ranges = Vec::new();
    for chunk in chunks {
        ranges.extend(chunk.iter().map(|letter| *letter..=*letter));
    }
    for pair in chunks.windows(2) {
        if let [low, high] = pair
            && let (Some(start), Some(end)) = (low.last(), high.first())
        {
            ranges.push(*start..=*end);
        }
    }
    ranges
}
