use betterlite_models::decisions::Json;
use indexmap::IndexMap;

#[derive(Clone)]
enum Child {
    Items,
    Property(String),
    Definition { key: &'static str, name: String },
    Union(&'static str),
}

struct Frame {
    shell: IndexMap<String, Json>,
    children: Vec<(Child, Json)>,
    next: usize,
    results: Vec<(Child, Json)>,
}

enum Job {
    Patch(Json),
    Close { frame: Frame, child: Child },
}

// LiteLLM recurses here; a deep schema would overflow the stack, so the walk keeps its own frames.
#[must_use]
pub fn add_additional_properties(schema: &Json) -> Json {
    let mut work = vec![Job::Patch(schema.clone())];
    let mut finished: Vec<Json> = Vec::new();
    while let Some(job) = work.pop() {
        match job {
            Job::Patch(node) => match open(&node) {
                None => finished.push(node),
                Some(frame) => advance(frame, &mut work, &mut finished),
            },
            Job::Close { mut frame, child } => {
                let Some(value) = finished.pop() else {
                    break;
                };
                frame.results.push((child, value));
                advance(frame, &mut work, &mut finished);
            }
        }
    }
    finished.pop().unwrap_or(Json::Null)
}

fn advance(mut frame: Frame, work: &mut Vec<Job>, finished: &mut Vec<Json>) {
    match frame.next_child() {
        Some((child, node)) => {
            work.push(Job::Close { frame, child });
            work.push(Job::Patch(node));
        }
        None => finished.push(frame.finish()),
    }
}

fn open(node: &Json) -> Option<Frame> {
    let Json::Object(original) = node else {
        return None;
    };
    let mut shell = original.clone();
    if matches!(original.get("type"), Some(Json::Text(kind)) if kind == "object")
        && !original.contains_key("additionalProperties")
    {
        shell.insert("additionalProperties".to_owned(), Json::Bool(false));
    }
    Some(Frame {
        children: children_of(original),
        shell,
        next: 0,
        results: Vec::new(),
    })
}

impl Frame {
    fn next_child(&mut self) -> Option<(Child, Json)> {
        let (child, value) = self.children.get(self.next)?;
        let pair = (child.clone(), value.clone());
        self.next = self.next.saturating_add(1);
        Some(pair)
    }

    fn finish(self) -> Json {
        let Self { shell, results, .. } = self;
        let mut fields = shell;
        let mut maps: IndexMap<&'static str, IndexMap<String, Json>> = IndexMap::new();
        let mut lists: IndexMap<&'static str, Vec<Json>> = IndexMap::new();
        let mut nested_items = None;
        for (child, value) in results {
            match child {
                Child::Items => {
                    nested_items = Some(value);
                }
                Child::Property(name) => {
                    maps.entry("properties").or_default().insert(name, value);
                }
                Child::Definition { key, name } => {
                    maps.entry(key).or_default().insert(name, value);
                }
                Child::Union(key) => {
                    lists.entry(key).or_default().push(value);
                }
            }
        }
        for (key, map) in maps {
            fields.insert(key.to_owned(), Json::Object(map));
        }
        for (key, list) in lists {
            fields.insert(key.to_owned(), Json::List(list));
        }
        if let Some(items) = nested_items {
            fields.insert("items".to_owned(), items);
        }
        Json::Object(fields)
    }
}

fn children_of(original: &IndexMap<String, Json>) -> Vec<(Child, Json)> {
    let mut children = Vec::new();
    push_items(&mut children, original);
    push_nested(&mut children, original);
    children
}

fn push_items(children: &mut Vec<(Child, Json)>, original: &IndexMap<String, Json>) {
    let Some(items @ Json::Object(_)) = original.get("items") else {
        return;
    };
    children.push((Child::Items, items.clone()));
}

fn push_nested(children: &mut Vec<(Child, Json)>, original: &IndexMap<String, Json>) {
    if let Some(Json::Object(properties)) = original.get("properties") {
        for (name, value) in properties {
            children.push((Child::Property(name.clone()), value.clone()));
        }
    }
    for key in ["$defs", "definitions"] {
        let Some(Json::Object(entries)) = original.get(key) else {
            continue;
        };
        for (name, value) in entries {
            children.push((
                Child::Definition {
                    key,
                    name: name.clone(),
                },
                value.clone(),
            ));
        }
    }
    for key in ["anyOf", "allOf", "oneOf"] {
        let Some(Json::List(items)) = original.get(key) else {
            continue;
        };
        for item in items {
            children.push((Child::Union(key), item.clone()));
        }
    }
}
