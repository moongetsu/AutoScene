use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::{CoreError, CoreResult};

/// Cheap, cloneable cancellation flag. Children cancel with their parent but
/// can also be cancelled on their own (e.g. one export item inside a batch).
#[derive(Clone, Debug, Default)]
pub struct CancelToken {
    own: Arc<AtomicBool>,
    parent: Option<Arc<AtomicBool>>,
}

impl CancelToken {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn child(&self) -> Self {
        Self { own: Arc::new(AtomicBool::new(false)), parent: Some(self.own.clone()) }
    }

    pub fn cancel(&self) {
        self.own.store(true, Ordering::Release);
    }

    pub fn is_cancelled(&self) -> bool {
        self.own.load(Ordering::Acquire) || self.parent.as_ref().is_some_and(|p| p.load(Ordering::Acquire))
    }

    /// `?`-friendly check for long loops.
    pub fn check(&self) -> CoreResult<()> {
        if self.is_cancelled() { Err(CoreError::Cancelled) } else { Ok(()) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn child_follows_parent() {
        let p = CancelToken::new();
        let c = p.child();
        assert!(!c.is_cancelled());
        p.cancel();
        assert!(c.is_cancelled());
        assert!(c.check().is_err());
    }

    #[test]
    fn child_cancel_does_not_touch_parent() {
        let p = CancelToken::new();
        let c = p.child();
        c.cancel();
        assert!(c.is_cancelled());
        assert!(!p.is_cancelled());
    }
}
