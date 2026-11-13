use std::convert::Infallible;
use std::mem::size_of;

use rand::TryRng;

use super::Sample;

// a sample is Python's 53-bit random() draw, so a wider draw scales the same fraction to its width
const SAMPLE_BITS: u32 = 53;
const U32_SCALE_DOWN: u32 = SAMPLE_BITS - u32::BITS;
const U64_SCALE_UP: u32 = u64::BITS - SAMPLE_BITS;
// the bits below a 53-bit draw are filled so a zero sample is never the value a range rejects
const U64_LOW_BITS: u64 = (1 << U64_SCALE_UP) - 1;

pub struct InjectedRandom<'samples> {
    pub samples: &'samples [Sample],
    pub next: usize,
    pub exhausted: bool,
}

impl<'samples> From<&'samples [Sample]> for InjectedRandom<'samples> {
    fn from(samples: &'samples [Sample]) -> Self {
        Self {
            samples,
            next: 0,
            exhausted: false,
        }
    }
}

impl InjectedRandom<'_> {
    // None marks the samples as exhausted, which the caller reports after the pick
    fn draw(&mut self) -> Option<u64> {
        if let Some(sample) = self.samples.get(self.next) {
            self.next = self.next.saturating_add(1);
            Some(sample.0)
        } else {
            self.exhausted = true;
            None
        }
    }
}

// a value past the samples is one no range rejects, so the sampler cannot loop before samples_exhausted
impl TryRng for InjectedRandom<'_> {
    type Error = Infallible;

    fn try_next_u32(&mut self) -> Result<u32, Self::Error> {
        let Some(sample) = self.draw() else {
            return Ok(u32::MAX);
        };
        let [first, second, third, fourth, ..] = (sample >> U32_SCALE_DOWN).to_le_bytes();
        // one is the smallest value every range accepts, so a zero sample still counts as one draw
        Ok(u32::from_le_bytes([first, second, third, fourth]).max(1))
    }

    fn try_next_u64(&mut self) -> Result<u64, Self::Error> {
        let Some(sample) = self.draw() else {
            return Ok(u64::MAX);
        };
        Ok((sample << U64_SCALE_UP) | U64_LOW_BITS)
    }

    fn try_fill_bytes(&mut self, dst: &mut [u8]) -> Result<(), Self::Error> {
        for chunk in dst.chunks_mut(size_of::<u64>()) {
            let bytes = self.draw().map_or_else(
                || u64::MAX.to_le_bytes(),
                |sample| ((sample << U64_SCALE_UP) | U64_LOW_BITS).to_le_bytes(),
            );
            for (slot, byte) in chunk.iter_mut().zip(bytes) {
                *slot = byte;
            }
        }
        Ok(())
    }
}
