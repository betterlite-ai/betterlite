use std::collections::{BTreeMap, BTreeSet};

const TOOL_USE_ID_FALLBACK: &str = "tool_use_id";

// One parse can yield several calls; the first keeps the id and the rest get __concat_N ids that no other call already owns.
pub(super) fn split_tool_call_ids(calls: &[(Option<&str>, usize)]) -> Vec<Vec<Option<String>>> {
    let mut taken = BTreeSet::new();
    for &(id, _) in calls {
        if let Some(call_id) = id.filter(|value| !value.is_empty()) {
            taken.insert(sanitize(call_id));
        }
    }
    let mut suffixes: BTreeMap<String, SuffixGenerator> = BTreeMap::new();
    for &(id, count) in calls {
        if let Some(call_id) = id.filter(|value| !value.is_empty())
            && count > 1
        {
            suffixes.insert(
                sanitize(call_id),
                SuffixGenerator {
                    call_id: call_id.to_owned(),
                    next: 1,
                },
            );
        }
    }
    calls
        .iter()
        .map(|&(id, count)| ids_for_call(id, count, &mut suffixes, &taken))
        .collect()
}

fn ids_for_call(
    id: Option<&str>,
    count: usize,
    suffixes: &mut BTreeMap<String, SuffixGenerator>,
    taken: &BTreeSet<String>,
) -> Vec<Option<String>> {
    let Some(call_id) = id.filter(|value| !value.is_empty()) else {
        return vec![None; count];
    };
    let mut ids = Vec::with_capacity(count);
    ids.push(Some(call_id.to_owned()));
    if count > 1
        && let Some(generator) = suffixes.get_mut(&sanitize(call_id))
    {
        for _ in 1..count {
            ids.push(Some(generator.next_id(taken)));
        }
    }
    ids
}

struct SuffixGenerator {
    call_id: String,
    next: u64,
}

impl SuffixGenerator {
    fn next_id(&mut self, taken: &BTreeSet<String>) -> String {
        loop {
            let call_id = self.call_id.as_str();
            let number = self.next;
            let candidate = format!("{call_id}__concat_{number}");
            self.next = self.next.saturating_add(1);
            if !taken.contains(&sanitize(&candidate)) {
                return candidate;
            }
        }
    }
}

// Anthropic's tool_use_id pattern is ^[a-zA-Z0-9_-]+$, and this rewrite only decides which generated __concat_N id is really free.
fn sanitize(tool_use_id: &str) -> String {
    if tool_use_id.is_empty() {
        return TOOL_USE_ID_FALLBACK.to_owned();
    }
    tool_use_id
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '_' | '-') {
                character
            } else {
                '_'
            }
        })
        .collect()
}
