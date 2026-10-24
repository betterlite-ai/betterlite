use crate::error::PathError;
use crate::paths::{Document, PathLimits, Segment, segments};

pub fn remove(limits: &PathLimits, document: &Document, path: &str) -> Result<Document, PathError> {
    let segments = segments::parse(limits, path)?;
    let mut result = document.clone();
    let mut pending = vec![(&mut result.json, 0_usize)];
    let mut visits = 0_usize;
    while let Some((node, position)) = pending.pop() {
        visits = visits.saturating_add(1);
        if visits > limits.max_visits {
            return Err(PathError::TooMuchWork {
                limit: limits.max_visits,
            });
        }
        let Some(segment) = segments.get(position) else {
            continue;
        };
        let next = position.saturating_add(1);
        let last = next == segments.len();
        // a list element is never deleted itself, so a trailing selector does nothing
        match segment {
            Segment::Field(name) if last => {
                node.as_object_mut().map(|object| object.remove(name));
            }
            Segment::Field(name) => pending.extend(
                node.as_object_mut()
                    .and_then(|object| object.get_mut(name))
                    .map(|child| (child, next)),
            ),
            Segment::Wildcard if !last => pending.extend(
                node.as_array_mut()
                    .into_iter()
                    .flatten()
                    .map(|item| (item, next)),
            ),
            Segment::Index(index) if !last => pending.extend(
                node.as_array_mut()
                    .and_then(|items| items.get_mut(*index))
                    .map(|element| (element, next)),
            ),
            Segment::Wildcard | Segment::Index(_) | Segment::Unmatched => {}
        }
    }
    Ok(result)
}

mod tests;
