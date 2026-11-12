#[path = "matcher/tests.rs"]
mod tests;

use std::collections::HashMap;

use rust_decimal::Decimal;

#[derive(Debug, Clone, Copy)]
struct Bounds {
    first_low: usize,
    first_high: usize,
    second_low: usize,
    second_high: usize,
}

#[must_use]
pub fn ratio(first: &[char], second: &[char]) -> Decimal {
    let total = first.len().saturating_add(second.len());
    if total == 0 {
        return Decimal::ONE;
    }
    Decimal::from(matching_characters(first, second).saturating_mul(2))
        .checked_div(Decimal::from(total))
        .unwrap_or(Decimal::ZERO)
}

#[must_use]
pub fn matching_characters(first: &[char], second: &[char]) -> usize {
    let positions = positions(second);
    let mut queue = vec![Bounds {
        first_low: 0,
        first_high: first.len(),
        second_low: 0,
        second_high: second.len(),
    }];
    let mut matched: usize = 0;
    while let Some(bounds) = queue.pop() {
        let (first_start, second_start, size) = longest_match(first, second, &positions, bounds);
        if size == 0 {
            continue;
        }
        matched = matched.saturating_add(size);
        if bounds.first_low < first_start && bounds.second_low < second_start {
            queue.push(Bounds {
                first_low: bounds.first_low,
                first_high: first_start,
                second_low: bounds.second_low,
                second_high: second_start,
            });
        }
        let first_end = first_start.saturating_add(size);
        let second_end = second_start.saturating_add(size);
        if first_end < bounds.first_high && second_end < bounds.second_high {
            queue.push(Bounds {
                first_low: first_end,
                first_high: bounds.first_high,
                second_low: second_end,
                second_high: bounds.second_high,
            });
        }
    }
    matched
}

fn positions(second: &[char]) -> HashMap<char, Vec<usize>> {
    let mut positions: HashMap<char, Vec<usize>> = HashMap::new();
    for (index, character) in second.iter().enumerate() {
        positions.entry(*character).or_default().push(index);
    }
    positions
}

// difflib's find_longest_match with no junk function: ties keep the earliest start in a, then in b
fn longest_match(
    first: &[char],
    second: &[char],
    positions: &HashMap<char, Vec<usize>>,
    bounds: Bounds,
) -> (usize, usize, usize) {
    let mut best = (bounds.first_low, bounds.second_low, 0);
    let mut previous_lengths: HashMap<usize, usize> = HashMap::new();
    for (index, character) in first
        .iter()
        .enumerate()
        .skip(bounds.first_low)
        .take(bounds.first_high.saturating_sub(bounds.first_low))
    {
        let mut lengths: HashMap<usize, usize> = HashMap::new();
        let found: &[usize] = positions.get(character).map_or(&[], Vec::as_slice);
        for &other in found
            .iter()
            .skip_while(|other| **other < bounds.second_low)
            .take_while(|other| **other < bounds.second_high)
        {
            let previous = other
                .checked_sub(1)
                .and_then(|previous| previous_lengths.get(&previous).copied())
                .unwrap_or(0);
            let size = previous.saturating_add(1);
            lengths.insert(other, size);
            if size > best.2 {
                best = (
                    index.saturating_add(1).saturating_sub(size),
                    other.saturating_add(1).saturating_sub(size),
                    size,
                );
            }
        }
        previous_lengths = lengths;
    }
    extend_match(first, second, bounds, best)
}

fn extend_match(
    first: &[char],
    second: &[char],
    bounds: Bounds,
    mut best: (usize, usize, usize),
) -> (usize, usize, usize) {
    while best.0 > bounds.first_low
        && best.1 > bounds.second_low
        && first.get(best.0.saturating_sub(1)) == second.get(best.1.saturating_sub(1))
    {
        best = (
            best.0.saturating_sub(1),
            best.1.saturating_sub(1),
            best.2.saturating_add(1),
        );
    }
    while best.0.saturating_add(best.2) < bounds.first_high
        && best.1.saturating_add(best.2) < bounds.second_high
        && first.get(best.0.saturating_add(best.2)) == second.get(best.1.saturating_add(best.2))
    {
        best.2 = best.2.saturating_add(1);
    }
    best
}
