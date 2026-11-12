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
    let mut blocks = Vec::new();
    while let Some(bounds) = queue.pop() {
        let (first_start, second_start, size) = longest_match(first, &positions, bounds);
        if size == 0 {
            continue;
        }
        blocks.push(size);
        enqueue_subregions(&mut queue, bounds, first_start, second_start, size);
    }
    blocks.iter().sum()
}

fn positions(second: &[char]) -> HashMap<char, Vec<usize>> {
    let mut positions: HashMap<char, Vec<usize>> = HashMap::new();
    for (index, character) in second.iter().enumerate() {
        positions.entry(*character).or_default().push(index);
    }
    positions
}

fn enqueue_subregions(
    queue: &mut Vec<Bounds>,
    bounds: Bounds,
    first_start: usize,
    second_start: usize,
    size: usize,
) {
    // the strictly-inside terms are redundant but stop a mutated longest_match from re-pushing the same bounds
    let left_gap = first_start
        .saturating_sub(bounds.first_low)
        .min(bounds.first_high.saturating_sub(first_start))
        .min(second_start.saturating_sub(bounds.second_low))
        .min(bounds.second_high.saturating_sub(second_start));
    if left_gap >= 1 {
        queue.push(Bounds {
            first_low: bounds.first_low,
            first_high: first_start,
            second_low: bounds.second_low,
            second_high: second_start,
        });
    }
    let first_end = first_start.saturating_add(size);
    let second_end = second_start.saturating_add(size);
    let right_gap = first_end
        .saturating_sub(bounds.first_low)
        .min(bounds.first_high.saturating_sub(first_end))
        .min(second_end.saturating_sub(bounds.second_low))
        .min(bounds.second_high.saturating_sub(second_end));
    if right_gap >= 1 {
        queue.push(Bounds {
            first_low: first_end,
            first_high: bounds.first_high,
            second_low: second_end,
            second_high: bounds.second_high,
        });
    }
}

// The backward and forward extension past junk is dead without a junk function, so difflib omits its effect.
fn longest_match(
    first: &[char],
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
    best
}
