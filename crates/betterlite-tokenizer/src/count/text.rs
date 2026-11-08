use std::cmp::Ordering;
use std::num::NonZeroUsize;
use std::ops::Range;

use crate::engine::Engine;
use crate::engine::limits::CountingService;

use super::error::CountError;

pub const EXTRAPOLATION_SAMPLES: usize = 16;

#[derive(Debug, Clone)]
pub struct Counted {
    pub engine: Engine,
    // an openai tokenizer chunks every string before encoding; a hugging face one counts the whole string
    pub chunks: Option<NonZeroUsize>,
}

pub struct Texter<'counter> {
    pub counter: &'counter CountingService,
    pub counted: &'counter Counted,
    pub max_exact_chars: NonZeroUsize,
}

impl Texter<'_> {
    pub async fn count(&self, text: &str) -> Result<usize, CountError> {
        let chars = text.chars().count();
        if chars <= self.max_exact_chars.get() {
            return self.exact(text).await;
        }
        self.sampled(text, chars).await
    }

    async fn exact(&self, text: &str) -> Result<usize, CountError> {
        let Some(chunk) = self.counted.chunks else {
            return Ok(self.counter.count(&self.counted.engine, text).await?);
        };
        let mut total = 0_usize;
        for slice in chunk_slices(text, chunk) {
            total = total
                .checked_add(self.counter.count(&self.counted.engine, slice).await?)
                .ok_or(CountError::Overflow)?;
        }
        Ok(total)
    }

    async fn sampled(&self, text: &str, chars: usize) -> Result<usize, CountError> {
        let mut tally = Tally::default();
        for range in sample_ranges(text, chars, self.max_exact_chars)? {
            let slice = text.get(range).ok_or(CountError::Boundary)?;
            let tokens = self.exact(slice).await?;
            tally.record(tokens, slice.chars().count())?;
        }
        let scalars = u128::try_from(chars).map_err(|_conversion| CountError::Overflow)?;
        let numerator = tally
            .tokens
            .checked_mul(scalars)
            .ok_or(CountError::Overflow)?;
        round_half_even(numerator, tally.scalars)
    }
}

// the sampled tokens and the sampled scalar values both feed one rounding step, so they grow together
#[derive(Debug, Clone, Copy, Default)]
struct Tally {
    tokens: u128,
    scalars: u128,
}

impl Tally {
    fn record(&mut self, tokens: usize, scalars: usize) -> Result<(), CountError> {
        let token_count = u128::try_from(tokens).map_err(|_conversion| CountError::Overflow)?;
        let scalar_count = u128::try_from(scalars).map_err(|_conversion| CountError::Overflow)?;
        self.tokens = self
            .tokens
            .checked_add(token_count)
            .ok_or(CountError::Overflow)?;
        self.scalars = self
            .scalars
            .checked_add(scalar_count)
            .ok_or(CountError::Overflow)?;
        Ok(())
    }
}

// Python slices the text by code points and tiktoken then encodes each slice on its own, so the chunk bounds are scalar indices
fn chunk_slices(text: &str, chunk: NonZeroUsize) -> Vec<&str> {
    let mut slices = Vec::new();
    let mut start = 0;
    let mut count = 0;
    for (index, _) in text.char_indices() {
        if count == chunk.get() {
            if let Some(slice) = text.get(start..index) {
                slices.push(slice);
            }
            start = index;
            count = 0;
        }
        count = count.saturating_add(1);
    }
    if start < text.len()
        && let Some(slice) = text.get(start..text.len())
    {
        slices.push(slice);
    }
    slices
}

fn sample_ranges(
    text: &str,
    chars: usize,
    max_exact_chars: NonZeroUsize,
) -> Result<Vec<Range<usize>>, CountError> {
    let total = max_exact_chars.get();
    let sample_count = EXTRAPOLATION_SAMPLES.min(total);
    let sample_chars = total
        .checked_div(sample_count)
        .ok_or(CountError::Overflow)?;
    let last_start = chars
        .checked_sub(sample_chars)
        .ok_or(CountError::Overflow)?;
    let divisor = sample_count.saturating_sub(1).max(1);
    let mut boundaries = Vec::with_capacity(sample_count.saturating_mul(2));
    for index in 0..sample_count {
        let start = scaled(last_start, index, divisor)?;
        boundaries.push(start);
        boundaries.push(
            start
                .checked_add(sample_chars)
                .ok_or(CountError::Overflow)?,
        );
    }
    let bytes = boundary_bytes(text, &boundaries, chars)?;
    let (pairs, rest) = bytes.as_chunks::<2>();
    if !rest.is_empty() {
        return Err(CountError::Boundary);
    }
    let mut ranges = Vec::with_capacity(sample_count);
    for [start, end] in pairs {
        ranges.push(*start..*end);
    }
    Ok(ranges)
}

fn scaled(last_start: usize, index: usize, divisor: usize) -> Result<usize, CountError> {
    let product = u128::try_from(last_start)
        .map_err(|_conversion| CountError::Overflow)?
        .checked_mul(u128::try_from(index).map_err(|_conversion| CountError::Overflow)?)
        .ok_or(CountError::Overflow)?;
    let quotient = product
        .checked_div(u128::try_from(divisor).map_err(|_conversion| CountError::Overflow)?)
        .ok_or(CountError::Overflow)?;
    usize::try_from(quotient).map_err(|_conversion| CountError::Overflow)
}

fn boundary_bytes(
    text: &str,
    boundaries: &[usize],
    chars: usize,
) -> Result<Vec<usize>, CountError> {
    let mut positions = text.char_indices().map(|(byte, _)| byte).enumerate();
    let mut current = positions.next();
    let mut bytes = Vec::with_capacity(boundaries.len());
    for boundary in boundaries {
        if *boundary > chars {
            return Err(CountError::Boundary);
        }
        while let Some((index, _)) = current {
            if index >= *boundary {
                break;
            }
            current = positions.next();
        }
        let byte = match current {
            Some((index, byte)) if index == *boundary => byte,
            None if *boundary == chars => text.len(),
            _ => return Err(CountError::Boundary),
        };
        bytes.push(byte);
    }
    Ok(bytes)
}

// Python's round() breaks a tie toward the even integer, on the exact quotient rather than a float
fn round_half_even(numerator: u128, denominator: u128) -> Result<usize, CountError> {
    let quotient = numerator
        .checked_div(denominator)
        .ok_or(CountError::Overflow)?;
    let remainder = numerator
        .checked_rem(denominator)
        .ok_or(CountError::Overflow)?;
    let doubled = remainder.checked_mul(2).ok_or(CountError::Overflow)?;
    let rounded = match doubled.cmp(&denominator) {
        Ordering::Less => quotient,
        Ordering::Greater => quotient.checked_add(1).ok_or(CountError::Overflow)?,
        Ordering::Equal => {
            let even = quotient.checked_rem(2).is_some_and(|value| value == 0);
            if even {
                quotient
            } else {
                quotient.checked_add(1).ok_or(CountError::Overflow)?
            }
        }
    };
    usize::try_from(rounded).map_err(|_conversion| CountError::Overflow)
}
