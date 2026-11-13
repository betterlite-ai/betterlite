use std::collections::BTreeSet;

use super::Difference;
use super::multipart::{
    captured_parts, first_part_difference, mask_generated_filenames, masked_boundary,
};
use crate::case::Compare;
use crate::mock::ProviderRequest;

mod tests;

const NO_CAPTURED_SELECTOR: &str =
    "compare.ignore names no captured provider request, part or header";
const REPEATED_SELECTOR: &str = "compare.ignore repeats a selector";
const BODIES_DIFFER: &str = "provider request bodies differ";
const GENERATED_FILENAMES_DIFFER: &str = "generated filenames differ in shape";

pub(crate) fn compare_provider_requests(
    left: &[ProviderRequest],
    right: &[ProviderRequest],
    compare: &Compare,
) -> Result<(), Difference> {
    if left.len() != right.len() {
        return Err(Difference {
            pointer: "/upstream".to_owned(),
            message: "provider request counts differ".to_owned(),
        });
    }
    let selectors: Vec<&str> = compare
        .ignore
        .iter()
        .filter(|ignored| ignored.pointer.starts_with("/upstream/"))
        .map(|ignored| ignored.pointer.as_str())
        .collect();
    // a case without selectors never pays for decoding the captures a second time
    if !selectors.is_empty() {
        check_selectors(&selectors, &rendered_pointers(left, right))?;
    }
    let selected: BTreeSet<&str> = selectors.into_iter().collect();
    for (index, (left_request, right_request)) in left.iter().zip(right).enumerate() {
        compare_request(index, left_request, right_request, &selected, compare)?;
    }
    Ok(())
}

// a selector matches only a pointer this walk renders, so no text under /upstream/ is parsed
fn check_selectors(selectors: &[&str], rendered: &BTreeSet<String>) -> Result<(), Difference> {
    let mut seen = BTreeSet::new();
    for pointer in selectors {
        if !seen.insert(*pointer) {
            return Err(Difference {
                pointer: (*pointer).to_owned(),
                message: REPEATED_SELECTOR.to_owned(),
            });
        }
    }
    for pointer in selectors {
        if !rendered.contains(*pointer) {
            return Err(Difference {
                pointer: (*pointer).to_owned(),
                message: NO_CAPTURED_SELECTOR.to_owned(),
            });
        }
    }
    Ok(())
}

fn rendered_pointers(left: &[ProviderRequest], right: &[ProviderRequest]) -> BTreeSet<String> {
    let mut rendered = BTreeSet::new();
    for (index, (left_request, right_request)) in left.iter().zip(right).enumerate() {
        rendered.insert(format!("/upstream/{index}/body"));
        for name in left_request.headers.keys() {
            if right_request.headers.contains_key(name) {
                rendered.insert(format!("/upstream/{index}/headers/{name}"));
            }
        }
        let (Some(left_parts), Some(right_parts)) =
            (captured_parts(left_request), captured_parts(right_request))
        else {
            continue;
        };
        for (part, (left_part, right_part)) in left_parts.iter().zip(&right_parts).enumerate() {
            if left_part.filename.is_some() && right_part.filename.is_some() {
                rendered.insert(format!("/upstream/{index}/parts/{part}/filename"));
            }
        }
    }
    rendered
}

fn compare_request(
    index: usize,
    left: &ProviderRequest,
    right: &ProviderRequest,
    selected: &BTreeSet<&str>,
    compare: &Compare,
) -> Result<(), Difference> {
    if left.method != right.method || left.path != right.path {
        return Err(Difference {
            pointer: format!("/upstream/{index}"),
            message: "provider request targets differ".to_owned(),
        });
    }
    if !selected.contains(format!("/upstream/{index}/body").as_str()) {
        compare_body(index, left, right, selected)?;
    }
    for header in &compare.upstream_headers {
        let name = header.to_ascii_lowercase();
        let pointer = format!("/upstream/{index}/headers/{name}");
        if selected.contains(pointer.as_str()) {
            continue;
        }
        let left_value = left.headers.get(&name).map(|value| masked_boundary(value));
        if left_value != right.headers.get(&name).map(|value| masked_boundary(value)) {
            return Err(Difference {
                pointer,
                message: "provider header values differ".to_owned(),
            });
        }
    }
    Ok(())
}

fn compare_body(
    index: usize,
    left: &ProviderRequest,
    right: &ProviderRequest,
    selected: &BTreeSet<&str>,
) -> Result<(), Difference> {
    let (Some(mut left_parts), Some(mut right_parts)) =
        (captured_parts(left), captured_parts(right))
    else {
        return if left.body == right.body {
            Ok(())
        } else {
            Err(Difference {
                pointer: format!("/upstream/{index}/body"),
                message: BODIES_DIFFER.to_owned(),
            })
        };
    };
    let count = left_parts.len().min(right_parts.len());
    let relaxed: Vec<usize> = (0..count)
        .filter(|part| {
            selected.contains(format!("/upstream/{index}/parts/{part}/filename").as_str())
        })
        .collect();
    if let Err(part) = mask_generated_filenames(&mut left_parts, &mut right_parts, &relaxed) {
        return Err(Difference {
            pointer: format!("/upstream/{index}/parts/{part}/filename"),
            message: GENERATED_FILENAMES_DIFFER.to_owned(),
        });
    }
    // two decoded captures compare part by part, so only their random boundaries may differ
    let Some(pointer) = first_part_difference(&left_parts, &right_parts) else {
        return Ok(());
    };
    Err(Difference {
        pointer: format!("/upstream/{index}{pointer}"),
        message: BODIES_DIFFER.to_owned(),
    })
}
