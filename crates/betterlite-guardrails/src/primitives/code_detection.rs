use regex::{Regex, RegexBuilder};

use super::error::PrimitiveError;

const CODE_PATTERNS: [(&str, &[&str]); 10] = [
    (
        "sql",
        &[
            r"\b(SELECT|INSERT|UPDATE|DELETE|DROP|CREATE|ALTER|TRUNCATE)\b.*\b(FROM|INTO|TABLE|SET|WHERE)\b",
            r"\b(SELECT)\s+[\w\*,\s]+\s+FROM\s+\w+",
            r"\b(INSERT\s+INTO|UPDATE\s+\w+\s+SET|DELETE\s+FROM)\b",
        ],
    ),
    (
        "python",
        &[
            r"^\s*(def|class|import|from|if|for|while|try|except|with)\s+",
            r"^\s*@\w+",
            r"\b(print|len|range|str|int|float|list|dict|set)\s*\(",
        ],
    ),
    (
        "javascript",
        &[
            r"\b(function|const|let|var|class|import|export)\s+",
            "=>",
            r"\b(console\.(log|error|warn))\s*\(",
        ],
    ),
    (
        "typescript",
        &[
            r":\s*(string|number|boolean|any|void|never)\b",
            r"\b(interface|type|enum)\s+\w+",
            r"<[A-Z]\w*>",
        ],
    ),
    (
        "java",
        &[
            r"\b(public|private|protected)\s+(static\s+)?(class|void|int|String)\b",
            r"\bSystem\.(out|err)\.print",
        ],
    ),
    (
        "go",
        &[r"\bfunc\s+\w+\s*\(", r"\b(package|import)\s+", ":="],
    ),
    (
        "rust",
        &[
            r"\b(fn|let|mut|impl|struct|enum|pub|mod)\s+",
            "->",
            r"\b(println!|format!)\s*\(",
        ],
    ),
    (
        "shell",
        &[
            r"^#!.*\b(bash|sh|zsh)\b",
            r"\b(echo|grep|sed|awk|cat|ls|cd|mkdir|rm)\s+",
            r"\$\{?\w+\}?",
        ],
    ),
    (
        "html",
        &[
            r"<\s*(html|head|body|div|span|p|a|img|script|style)\b[^>]*>",
            r"</\s*(html|head|body|div|span|p|a|script|style)\s*>",
        ],
    ),
    (
        "css",
        &[
            r"\{[^}]*:\s*[^}]+;[^}]*\}",
            r"@(media|keyframes|import|font-face)\b",
        ],
    ),
];

pub struct CodeDetector {
    languages: Vec<(&'static str, Vec<Regex>)>,
}

impl CodeDetector {
    pub fn new() -> Result<Self, PrimitiveError> {
        let mut languages = Vec::new();
        for (language, patterns) in CODE_PATTERNS {
            let mut compiled = Vec::new();
            for pattern in patterns {
                let regex = RegexBuilder::new(pattern)
                    .case_insensitive(true)
                    .multi_line(true)
                    .build()
                    .map_err(|source| PrimitiveError::CodePattern { language, source })?;
                compiled.push(regex);
            }
            languages.push((language, compiled));
        }
        Ok(Self { languages })
    }

    #[must_use]
    pub fn detect_code_languages(&self, text: &str) -> Vec<&'static str> {
        self.languages
            .iter()
            .filter(|(_, patterns)| patterns.iter().any(|pattern| pattern.is_match(text)))
            .map(|(language, _)| *language)
            .collect()
    }

    #[must_use]
    pub fn contains_code_language(&self, text: &str, languages: &[String]) -> bool {
        let detected = self.detect_code_languages(text);
        languages.iter().any(|wanted| {
            detected
                .iter()
                .any(|found| found.eq_ignore_ascii_case(wanted))
        })
    }
}
