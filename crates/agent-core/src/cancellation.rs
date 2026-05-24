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

#[cfg(test)]
mod tests {
    use std::{
        sync::{Arc, Barrier},
        thread,
    };

    use super::CancellationToken;

    #[test]
    fn cancellation_token_check_returns_error_after_cancel() {
        let token = CancellationToken::new();
        token.check().expect("fresh token should not be canceled");

        token.cancel(" stop now ");

        let error = token.check().expect_err("canceled token should fail");
        assert_eq!(error.reason(), "stop now");
        assert_eq!(token.reason().as_deref(), Some("stop now"));
    }

    #[test]
    fn cancellation_token_clone_shares_state() {
        let token = CancellationToken::new();
        let cloned = token.clone();

        cloned.cancel("from clone");

        assert!(token.is_canceled());
        assert_eq!(token.reason().as_deref(), Some("from clone"));
        assert_eq!(cloned.reason().as_deref(), Some("from clone"));
    }

    #[test]
    fn cancellation_token_keeps_first_recorded_reason() {
        let token = CancellationToken::new();

        token.cancel("first");
        token.cancel("second");

        assert_eq!(token.reason().as_deref(), Some("first"));
    }

    #[test]
    fn cancellation_token_concurrent_cancels_store_single_stable_reason() {
        let token = CancellationToken::new();
        let barrier = Arc::new(Barrier::new(9));
        let mut handles = Vec::new();

        for index in 0..8 {
            let token = token.clone();
            let barrier = Arc::clone(&barrier);
            handles.push(thread::spawn(move || {
                barrier.wait();
                token.cancel(format!("reason-{index}"));
            }));
        }

        barrier.wait();
        for handle in handles {
            handle.join().expect("cancel thread should join");
        }

        let reason = token.reason().expect("one reason should be recorded");
        assert!(
            (0..8).any(|index| reason == format!("reason-{index}")),
            "unexpected cancellation reason: {reason}"
        );

        token.cancel("late");
        assert_eq!(token.reason().as_deref(), Some(reason.as_str()));
    }
}
