mod tests;

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use itertools::Itertools as _;
use serde::Deserialize;
use strum::IntoStaticStr;

use crate::development::Development;
use crate::error::GateError;
use crate::git::git;
use crate::spec::backlog::{BacklogEntry, Resolution};

const REPOSITORY: &str = "https://github.com/betterlite-ai/betterlite";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, IntoStaticStr)]
#[strum(serialize_all = "lowercase")]
enum Kind {
    Expects,
    Lines,
    Advisory,
    Duplicate,
}

struct Row {
    kind: Kind,
    place: String,
    value: String,
    default: String,
    reason: String,
    link: String,
    date: String,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct BudgetNote {
    pub(crate) task: String,
    pub(crate) commit: String,
    pub(crate) reason: String,
}

impl BudgetNote {
    fn add(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        if !self.reason.is_empty() {
            self.reason.push(' ');
        }
        self.reason.push_str(text);
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Blamed {
    pub(crate) line: String,
    pub(crate) commit: String,
    pub(crate) summary: String,
}

#[derive(Deserialize)]
struct Planned {
    budget: PlannedBudget,
}

#[derive(Deserialize)]
struct PlannedBudget {
    max_changed_lines: usize,
}

#[derive(Deserialize)]
struct Deny {
    advisories: Advisories,
    bans: Bans,
}

#[derive(Deserialize)]
struct Advisories {
    #[serde(default)]
    ignore: Vec<Listed>,
}

#[derive(Deserialize)]
struct Bans {
    #[serde(default)]
    skip: Vec<Listed>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum Listed {
    Bare(String),
    Advisory {
        id: String,
        reason: Option<String>,
    },
    Crate {
        #[serde(rename = "crate")]
        name: String,
        reason: Option<String>,
    },
}

pub(crate) fn budget_notes(log: &str) -> Vec<BudgetNote> {
    let mut notes = Vec::new();
    for record in log.split('\u{1e}') {
        let mut lines = record.lines();
        let Some(commit) = lines.next() else {
            continue;
        };
        let mut current: Option<BudgetNote> = None;
        for line in lines {
            let task = line
                .trim_start_matches("* ")
                .strip_prefix("docs(tasks): ")
                .and_then(|rest| rest.split_once(" budget"))
                .map(|(id, _)| id);
            if task.is_some() || line.starts_with("* ") {
                notes.extend(current.take());
            }
            if let Some(id) = task {
                current = Some(BudgetNote {
                    task: id.to_owned(),
                    commit: commit.to_owned(),
                    reason: String::new(),
                });
                continue;
            }
            if let Some(note) = current.as_mut() {
                note.add(line.trim());
            }
        }
        notes.extend(current);
    }
    notes
}

pub(crate) fn pull_number(subject: &str) -> Option<u32> {
    let (_, number) = subject.strip_suffix(')')?.rsplit_once("(#")?;
    number.parse().ok()
}

pub(crate) fn merge_dates(log: &str) -> BTreeMap<u32, String> {
    let mut dates = BTreeMap::new();
    for (date, subject) in log.lines().filter_map(|line| line.split_once(' ')) {
        if let Some(number) = pull_number(subject) {
            dates.entry(number).or_insert_with(|| date.to_owned());
        }
    }
    dates
}

pub(crate) fn blamed_lines(porcelain: &str) -> Vec<Blamed> {
    let mut blamed = Vec::new();
    let mut commit = "";
    let mut summary = "";
    for line in porcelain.lines() {
        if let Some(content) = line.strip_prefix('\t') {
            blamed.push(Blamed {
                line: content.to_owned(),
                commit: commit.to_owned(),
                summary: summary.to_owned(),
            });
            continue;
        }
        if let Some(text) = line.strip_prefix("summary ") {
            summary = text;
            continue;
        }
        if let Some(hash) = line.split(' ').next()
            && hash.len() == 40
            && hash.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            commit = hash;
        }
    }
    blamed
}

pub(crate) fn csv_line(cells: &[&str]) -> String {
    cells
        .iter()
        .map(|cell| format!("\"{}\"", cell.replace('"', "\"\"")))
        .join(",")
}

fn planned_lines(
    development: &Development,
    commit: &str,
    file: &str,
) -> Result<Option<usize>, GateError> {
    let path = format!(".development/{file}");
    let parent = format!("{commit}^");
    let listed = git(
        &development.root,
        &["ls-tree", "--name-only", &parent, "--", &path],
    )?;
    if listed.trim().is_empty() {
        return Ok(None);
    }
    let spot = format!("{parent}:{path}");
    let text = git(&development.root, &["show", &spot])?;
    let planned: Planned = serde_saphyr::from_str(&text).map_err(|source| GateError::Yaml {
        path: PathBuf::from(spot),
        source: Box::new(source),
    })?;
    Ok(Some(planned.budget.max_changed_lines))
}

fn task_rows(
    development: &Development,
    entry: &BacklogEntry,
    notes: &[BudgetNote],
    dates: &BTreeMap<u32, String>,
) -> Result<Vec<Row>, GateError> {
    let (Some(file), Some(pr), Some(Resolution::Merged)) =
        (&entry.file, entry.pr, entry.resolution)
    else {
        return Ok(Vec::new());
    };
    let budget = development.task(&entry.id)?.budget;
    let given = notes
        .iter()
        .map(|note| note.reason.as_str())
        .filter(|text| !text.is_empty())
        .join(" ");
    let reason = match (notes.is_empty(), given.is_empty()) {
        (true, _) => "planned with the task".to_owned(),
        (false, true) => "the budget commit gives no reason".to_owned(),
        (false, false) => given,
    };
    let row = |kind, value: usize, default: usize| Row {
        kind,
        place: entry.id.clone(),
        value: value.to_string(),
        default: default.to_string(),
        reason: reason.clone(),
        link: format!("{REPOSITORY}/pull/{pr}"),
        date: dates.get(&pr).cloned().unwrap_or_default(),
    };
    let mut rows = Vec::new();
    if budget.max_new_expects > 0 {
        rows.push(row(Kind::Expects, budget.max_new_expects, 0));
    }
    if let Some(oldest) = notes.last()
        && let Some(planned) = planned_lines(development, &oldest.commit, file)?
        && planned < budget.max_changed_lines
    {
        rows.push(row(Kind::Lines, budget.max_changed_lines, planned));
    }
    Ok(rows)
}

fn budget_rows(development: &Development) -> Result<Vec<Row>, GateError> {
    let root = &development.root;
    let merges = git(root, &["log", "--first-parent", "--format=%cs %s", "HEAD"])?;
    let dates = merge_dates(&merges);
    let pattern = r"--grep=^(\* )?docs\(tasks\): B[0-9]+ budget";
    let log = git(
        root,
        &[
            "log",
            "--extended-regexp",
            pattern,
            "--format=%x1e%H%n%s%n%b",
            "HEAD",
        ],
    )?;
    let mut notes: BTreeMap<String, Vec<BudgetNote>> = BTreeMap::new();
    for note in budget_notes(&log) {
        notes.entry(note.task.clone()).or_default().push(note);
    }
    let mut rows = Vec::new();
    for entry in &development.backlog.tasks {
        let own = notes.get(&entry.id).map_or(&[][..], Vec::as_slice);
        rows.extend(task_rows(development, entry, own, &dates)?);
    }
    Ok(rows)
}

fn deny_rows(development: &Development) -> Result<Vec<Row>, GateError> {
    let path = development.root.join("deny.toml");
    let text = fs::read_to_string(&path).map_err(|source| GateError::Read {
        path: path.clone(),
        source,
    })?;
    let deny: Deny = toml::from_str(&text).map_err(|source| GateError::Toml {
        path,
        source: Box::new(source),
    })?;
    let porcelain = git(
        &development.root,
        &["blame", "--line-porcelain", "--", "deny.toml"],
    )?;
    let blamed = blamed_lines(&porcelain);
    let history = git(
        &development.root,
        &["log", "--format=%H %cs", "--", "deny.toml"],
    )?;
    let dates: BTreeMap<&str, &str> = history
        .lines()
        .filter_map(|line| line.split_once(' '))
        .collect();
    let advisories = deny.advisories.ignore.iter();
    let listed = (advisories.map(|item| (Kind::Advisory, item)))
        .chain(deny.bans.skip.iter().map(|item| (Kind::Duplicate, item)));
    Ok(listed
        .map(|(kind, item)| deny_row(kind, item, &blamed, &dates))
        .collect())
}

fn deny_row(kind: Kind, item: &Listed, blamed: &[Blamed], dates: &BTreeMap<&str, &str>) -> Row {
    let (name, reason) = match item {
        Listed::Bare(name) => (name, None),
        Listed::Advisory { id, reason } => (id, reason.as_ref()),
        Listed::Crate { name, reason } => (name, reason.as_ref()),
    };
    let quoted = format!("\"{name}\"");
    let origin = blamed.iter().find(|line| line.line.contains(&quoted));
    let summary = origin.map(|line| line.summary.as_str()).unwrap_or_default();
    let commit = origin.map(|line| line.commit.as_str()).unwrap_or_default();
    let link = match (kind, pull_number(summary)) {
        (Kind::Advisory, _) => format!("https://rustsec.org/advisories/{name}.html"),
        (_, Some(number)) => format!("{REPOSITORY}/pull/{number}"),
        (_, None) => format!("{REPOSITORY}/commit/{commit}"),
    };
    Row {
        kind,
        place: name.clone(),
        value: "allowed".to_owned(),
        default: "denied".to_owned(),
        reason: reason.map_or_else(|| summary.to_owned(), Clone::clone),
        link,
        date: dates
            .get(commit)
            .map(|date| (*date).to_owned())
            .unwrap_or_default(),
    }
}

pub(crate) fn exceptions(development: &Development) -> Result<Vec<String>, GateError> {
    let mut rows = budget_rows(development)?;
    rows.extend(deny_rows(development)?);
    rows.sort_by(|left, right| {
        left.kind
            .cmp(&right.kind)
            .then_with(|| right.date.cmp(&left.date))
    });
    let mut lines = vec!["kind,where,value,default,reason,link,date".to_owned()];
    for row in &rows {
        let kind = <&str>::from(row.kind);
        lines.push(csv_line(&[
            kind,
            &row.place,
            &row.value,
            &row.default,
            &row.reason,
            &row.link,
            &row.date,
        ]));
    }
    Ok(lines)
}
