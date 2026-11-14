pub trait RandomBytes: Send + Sync {
    fn fill(&self, buffer: &mut [u8]) -> Result<(), getrandom::Error>;
}

pub struct SystemRandom;

impl RandomBytes for SystemRandom {
    fn fill(&self, buffer: &mut [u8]) -> Result<(), getrandom::Error> {
        getrandom::fill(buffer)
    }
}
