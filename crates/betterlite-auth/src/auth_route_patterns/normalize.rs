use std::collections::VecDeque;
use std::sync::Mutex;

use regex::Regex;

const CACHE_CAPACITY: usize = 256;
const ROOT_TOKEN: &str = "__ROOT__";
const RESPONSE: &[(&str, &str)] = &[
    (
        "^(__ROOT__/responses)/([^/]+)(/input_items)$",
        "$1/{response_id}$3",
    ),
    (
        "^(__ROOT__/responses)/([^/]+)(/cancel)$",
        "$1/{response_id}$3",
    ),
    ("^(__ROOT__/responses)/([^/]+)$", "$1/{response_id}"),
];
const THREADS: &[(&str, &str)] = &[
    (
        "^(__ROOT__/threads)/([^/]+)(/runs)/([^/]+)(/steps)/([^/]+)$",
        "$1/{thread_id}$3/{run_id}$5/{step_id}",
    ),
    (
        "^(__ROOT__/threads)/([^/]+)(/runs)/([^/]+)(/steps)$",
        "$1/{thread_id}$3/{run_id}$5",
    ),
    (
        "^(__ROOT__/threads)/([^/]+)(/runs)/([^/]+)(/cancel)$",
        "$1/{thread_id}$3/{run_id}$5",
    ),
    (
        "^(__ROOT__/threads)/([^/]+)(/runs)/([^/]+)(/submit_tool_outputs)$",
        "$1/{thread_id}$3/{run_id}$5",
    ),
    (
        "^(__ROOT__/threads)/([^/]+)(/runs)/([^/]+)$",
        "$1/{thread_id}$3/{run_id}",
    ),
    ("^(__ROOT__/threads)/([^/]+)(/runs)$", "$1/{thread_id}$3"),
    (
        "^(__ROOT__/threads)/([^/]+)(/messages)/([^/]+)$",
        "$1/{thread_id}$3/{message_id}",
    ),
    (
        "^(__ROOT__/threads)/([^/]+)(/messages)$",
        "$1/{thread_id}$3",
    ),
    ("^(__ROOT__/threads)/([^/]+)$", "$1/{thread_id}"),
];
const VECTOR_STORES: &[(&str, &str)] = &[
    (
        "^(__ROOT__/vector_stores)/([^/]+)(/files)/([^/]+)$",
        "$1/{vector_store_id}$3/{file_id}",
    ),
    (
        "^(__ROOT__/vector_stores)/([^/]+)(/files)$",
        "$1/{vector_store_id}$3",
    ),
    (
        "^(__ROOT__/vector_stores)/([^/]+)(/file_batches)/([^/]+)$",
        "$1/{vector_store_id}$3/{batch_id}",
    ),
    (
        "^(__ROOT__/vector_stores)/([^/]+)(/file_batches)$",
        "$1/{vector_store_id}$3",
    ),
    ("^(__ROOT__/vector_stores)/([^/]+)$", "$1/{vector_store_id}"),
];
const ASSISTANTS: &[(&str, &str)] = &[("^(__ROOT__/assistants)/([^/]+)$", "$1/{assistant_id}")];
const FILES: &[(&str, &str)] = &[
    ("^(__ROOT__/files)/([^/]+)(/content)$", "$1/{file_id}$3"),
    ("^(__ROOT__/files)/([^/]+)$", "$1/{file_id}"),
];
const BATCHES: &[(&str, &str)] = &[
    ("^(__ROOT__/batches)/([^/]+)(/cancel)$", "$1/{batch_id}$3"),
    ("^(__ROOT__/batches)/([^/]+)$", "$1/{batch_id}"),
];
const FINE_TUNING: &[(&str, &str)] = &[
    (
        "^(__ROOT__/fine_tuning/jobs)/([^/]+)(/events)$",
        "$1/{fine_tuning_job_id}$3",
    ),
    (
        "^(__ROOT__/fine_tuning/jobs)/([^/]+)(/cancel)$",
        "$1/{fine_tuning_job_id}$3",
    ),
    (
        "^(__ROOT__/fine_tuning/jobs)/([^/]+)(/checkpoints)$",
        "$1/{fine_tuning_job_id}$3",
    ),
    (
        "^(__ROOT__/fine_tuning/jobs)/([^/]+)$",
        "$1/{fine_tuning_job_id}",
    ),
];
const MODELS: &[(&str, &str)] = &[("^(__ROOT__/models)/([^/]+)$", "$1/{model}")];

pub struct RouteNormalizer {
    patterns: Vec<(Regex, String)>,
    cache: Mutex<VecDeque<(String, String)>>,
}

impl Default for RouteNormalizer {
    fn default() -> Self {
        let mut patterns = Vec::new();
        for root in ["/v1", "/openai/v1"] {
            for group in [
                RESPONSE,
                THREADS,
                VECTOR_STORES,
                ASSISTANTS,
                FILES,
                BATCHES,
                FINE_TUNING,
                MODELS,
            ] {
                append_patterns(&mut patterns, root, group);
            }
        }
        append_patterns(
            &mut patterns,
            "",
            &[
                ("^(/responses)/([^/]+)(/input_items)$", "$1/{response_id}$3"),
                ("^(/responses)/([^/]+)(/cancel)$", "$1/{response_id}$3"),
                ("^(/responses)/([^/]+)$", "$1/{response_id}"),
            ],
        );
        Self {
            patterns,
            cache: Mutex::new(VecDeque::new()),
        }
    }
}

fn append_patterns(compiled: &mut Vec<(Regex, String)>, root: &str, group: &[(&str, &str)]) {
    for (template, replacement) in group {
        let pattern = template.replace(ROOT_TOKEN, root);
        if let Ok(regex) = Regex::new(&pattern) {
            compiled.push((regex, (*replacement).to_owned()));
        }
    }
}

impl RouteNormalizer {
    #[must_use]
    pub fn normalize_request_route(&self, route: &str) -> String {
        if let Ok(mut entries) = self.cache.lock()
            && let Some(position) = entries.iter().position(|(key, _)| key == route)
            && let Some(entry) = entries.remove(position)
        {
            let value = entry.1.clone();
            entries.push_front(entry);
            return value;
        }
        let normalized = self
            .patterns
            .iter()
            .find_map(|(pattern, replacement)| {
                let value = pattern.replace(route, replacement.as_str()).into_owned();
                (value != route).then_some(value)
            })
            .unwrap_or_else(|| route.to_owned());
        if let Ok(mut entries) = self.cache.lock() {
            entries.push_front((route.to_owned(), normalized.clone()));
            entries.truncate(CACHE_CAPACITY);
        }
        normalized
    }
}
