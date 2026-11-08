use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Clone, Debug, Default)]
pub struct LensAnalysis {
    active: Arc<AtomicBool>,
}

impl LensAnalysis {
    #[must_use]
    pub fn is_lens_analysis(&self) -> bool {
        // the scope lives on this injected context, so every caller sharing the context sees it
        self.active.load(Ordering::SeqCst)
    }

    #[must_use]
    pub fn lens_analysis(&self) -> LensAnalysisScope {
        let previous = self.active.swap(true, Ordering::SeqCst);
        LensAnalysisScope {
            active: Arc::clone(&self.active),
            previous,
        }
    }
}

pub struct LensAnalysisScope {
    active: Arc<AtomicBool>,
    previous: bool,
}

impl Drop for LensAnalysisScope {
    fn drop(&mut self) {
        // the token reset in context.py restores the value a nested scope found
        self.active.store(self.previous, Ordering::SeqCst);
    }
}

mod tests;
