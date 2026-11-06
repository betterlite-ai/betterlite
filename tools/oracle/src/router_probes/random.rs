use std::convert::Infallible;
use std::mem::size_of;

use rand::TryRng;

use super::Sample;

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
        let value = self.draw();
        Ok(u32::try_from(value).unwrap_or_default())
    }

    fn try_next_u64(&mut self) -> Result<u64, Self::Error> {
        Ok(self.draw())
    }

    fn try_fill_bytes(&mut self, dst: &mut [u8]) -> Result<(), Self::Error> {
        for chunk in dst.chunks_mut(size_of::<u64>()) {
            for (slot, byte) in chunk.iter_mut().zip(self.draw().to_le_bytes()) {
                *slot = byte;
            }
        }
        Ok(())
    }
}
