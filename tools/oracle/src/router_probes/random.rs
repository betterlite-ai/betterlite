use std::convert::Infallible;
use std::mem::size_of;

use rand::TryRng;

use super::Sample;

// a sample is Python's 53-bit random() draw, so a wider draw scales the same fraction to its width
const SAMPLE_BITS: u32 = 53;
const U32_SCALE_DOWN: u32 = SAMPLE_BITS - u32::BITS;
const U64_SCALE_UP: u32 = u64::BITS - SAMPLE_BITS;

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
    // a draw past the samples still answers, so the caller reports samples_exhausted after the pick
    fn draw(&mut self) -> u64 {
        if let Some(sample) = self.samples.get(self.next) {
            self.next = self.next.saturating_add(1);
            sample.0
        } else {
            self.exhausted = true;
            0
        }
    }
}

impl TryRng for InjectedRandom<'_> {
    type Error = Infallible;

    fn try_next_u32(&mut self) -> Result<u32, Self::Error> {
        let [first, second, third, fourth, ..] = (self.draw() >> U32_SCALE_DOWN).to_le_bytes();
        Ok(u32::from_le_bytes([first, second, third, fourth]))
    }

    fn try_next_u64(&mut self) -> Result<u64, Self::Error> {
        Ok(self.draw() << U64_SCALE_UP)
    }

    fn try_fill_bytes(&mut self, dst: &mut [u8]) -> Result<(), Self::Error> {
        for chunk in dst.chunks_mut(size_of::<u64>()) {
            for (slot, byte) in chunk
                .iter_mut()
                .zip((self.draw() << U64_SCALE_UP).to_le_bytes())
            {
                *slot = byte;
            }
        }
        Ok(())
    }
}
