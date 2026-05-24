use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

use thiserror::Error;

#[derive(Debug, Clone)]
pub struct CancellationToken {
    inner: Arc<CancellationState>,
}

#[derive(Debug)]
struct CancellationState {
    canceled: AtomicBool,
    reason: Mutex<Option<String>>,
}

impl CancellationToken {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(CancellationState {
                canceled: AtomicBool::new(false),
                reason: Mutex::new(None),
            }),
        }
    }

    pub fn cancel(&self, reason: impl Into<String>) {
        let reason = normalize_reason(reason.into());
        {
            let mut stored_reason = self.lock_reason();
            if stored_reason.is_none() {
                *stored_reason = Some(reason);
            }
        }
        self.inner.canceled.store(true, Ordering::SeqCst);
    }

    pub fn is_canceled(&self) -> bool {
        self.inner.canceled.load(Ordering::SeqCst)
    }

    pub fn reason(&self) -> Option<String> {
        self.lock_reason().clone()
    }

    pub fn cancellation_reason(&self) -> String {
        self.reason()
            .unwrap_or_else(|| "operation canceled".to_owned())
    }

    pub fn check(&self) -> Result<(), CancellationError> {
        if self.is_canceled() {
            return Err(CancellationError {
                reason: self.cancellation_reason(),
            });
        }

        Ok(())
    }

    fn lock_reason(&self) -> std::sync::MutexGuard<'_, Option<String>> {
        self.inner
            .reason
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

impl Default for CancellationToken {
    fn default() -> Self {
        Self::new()
    }
}

impl PartialEq for CancellationToken {
    fn eq(&self, other: &Self) -> bool {
        self.is_canceled() == other.is_canceled() && self.reason() == other.reason()
    }
}

impl Eq for CancellationToken {}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("operation canceled: {reason}")]
pub struct CancellationError {
    reason: String,
}

impl CancellationError {
    pub fn reason(&self) -> &str {
        &self.reason
    }
}

fn normalize_reason(reason: String) -> String {
    let trimmed = reason.trim();
    if trimmed.is_empty() {
        "operation canceled".to_owned()
    } else {
        trimmed.to_owned()
    }
}
