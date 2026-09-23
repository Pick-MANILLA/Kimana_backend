//! A generic circuit breaker for calls to flaky external-ish dependencies —
//! an FX rate feed, a banking partner API, anything reachable over a
//! network. After enough consecutive failures it trips open and fails fast,
//! without even attempting the call, for a cooldown period; then it lets a
//! single trial call through to test whether the dependency has recovered.
//!
//! This type doesn't know what it's protecting — `call` takes any fallible
//! async closure — so the same breaker shape works for a future real HTTP
//! partner integration, not just the FX feed that uses it today.

use std::future::Future;
use std::sync::Mutex;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy)]
enum State {
    Closed,
    Open { opened_at: Instant },
    HalfOpen,
}

/// Why `CircuitBreaker::call` didn't return a value from `op`.
#[derive(Debug)]
pub enum CallError<E> {
    /// The breaker was open and `op` was never attempted.
    Open { retry_after: Duration },
    /// `op` was attempted and returned an error.
    Failed(E),
}

pub struct CircuitBreaker {
    state: Mutex<State>,
    consecutive_failures: Mutex<u32>,
    failure_threshold: u32,
    reset_timeout: Duration,
}

impl CircuitBreaker {
    /// `failure_threshold` consecutive failures trips the breaker open;
    /// it stays open for `reset_timeout` before allowing a trial call.
    pub fn new(failure_threshold: u32, reset_timeout: Duration) -> Self {
        Self {
            state: Mutex::new(State::Closed),
            consecutive_failures: Mutex::new(0),
            failure_threshold: failure_threshold.max(1),
            reset_timeout,
        }
    }

    /// Runs `op` unless the breaker is open and its cooldown hasn't
    /// elapsed yet, in which case `op` is never called at all.
    pub async fn call<F, Fut, T, E>(&self, op: F) -> Result<T, CallError<E>>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<T, E>>,
    {
        // Was this attempt a half-open trial? Determines whether a failure
        // re-opens the breaker immediately, bypassing the failure count.
        let is_trial = {
            let mut state = self.state.lock().unwrap();
            match *state {
                State::Open { opened_at } => {
                    let elapsed = opened_at.elapsed();
                    if elapsed < self.reset_timeout {
                        return Err(CallError::Open {
                            retry_after: self.reset_timeout - elapsed,
                        });
                    }
                    *state = State::HalfOpen;
                    true
                }
                State::HalfOpen => true,
                State::Closed => false,
            }
        };

        match op().await {
            Ok(value) => {
                *self.consecutive_failures.lock().unwrap() = 0;
                *self.state.lock().unwrap() = State::Closed;
                Ok(value)
            }
            Err(err) => {
                let mut failures = self.consecutive_failures.lock().unwrap();
                *failures += 1;
                if is_trial || *failures >= self.failure_threshold {
                    *self.state.lock().unwrap() = State::Open {
                        opened_at: Instant::now(),
                    };
                }
                Err(CallError::Failed(err))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    async fn ok() -> Result<&'static str, &'static str> {
        Ok("fine")
    }

    async fn fail() -> Result<&'static str, &'static str> {
        Err("boom")
    }

    #[tokio::test]
    async fn closed_breaker_passes_calls_through() {
        let breaker = CircuitBreaker::new(3, Duration::from_secs(10));
        assert!(matches!(breaker.call(ok).await, Ok("fine")));
    }

    #[tokio::test]
    async fn single_failure_stays_closed() {
        let breaker = CircuitBreaker::new(3, Duration::from_secs(10));
        assert!(matches!(
            breaker.call(fail).await,
            Err(CallError::Failed(_))
        ));
        // Below threshold: the next call is still attempted, not fast-failed.
        assert!(matches!(breaker.call(ok).await, Ok("fine")));
    }

    #[tokio::test]
    async fn opens_after_consecutive_failures_reach_threshold() {
        let breaker = CircuitBreaker::new(3, Duration::from_secs(10));
        for _ in 0..3 {
            assert!(matches!(
                breaker.call(fail).await,
                Err(CallError::Failed(_))
            ));
        }
        let calls = AtomicU32::new(0);
        let result = breaker
            .call(|| async {
                calls.fetch_add(1, Ordering::SeqCst);
                ok().await
            })
            .await;
        assert!(matches!(result, Err(CallError::Open { .. })));
        assert_eq!(
            calls.load(Ordering::SeqCst),
            0,
            "op must not run while open"
        );
    }

    #[tokio::test]
    async fn half_open_trial_success_fully_closes_breaker() {
        let breaker = CircuitBreaker::new(2, Duration::from_millis(20));
        for _ in 0..2 {
            let _ = breaker.call(fail).await;
        }
        assert!(matches!(
            breaker.call(ok).await,
            Err(CallError::Open { .. })
        ));

        tokio::time::sleep(Duration::from_millis(30)).await;
        assert!(
            matches!(breaker.call(ok).await, Ok("fine")),
            "trial call must be attempted after cooldown"
        );

        // Failure count was reset by the successful trial: one failure
        // alone must not immediately re-open the breaker.
        assert!(matches!(
            breaker.call(fail).await,
            Err(CallError::Failed(_))
        ));
        assert!(matches!(breaker.call(ok).await, Ok("fine")));
    }

    #[tokio::test]
    async fn half_open_trial_failure_reopens_immediately() {
        let breaker = CircuitBreaker::new(2, Duration::from_millis(20));
        for _ in 0..2 {
            let _ = breaker.call(fail).await;
        }
        tokio::time::sleep(Duration::from_millis(30)).await;

        assert!(
            matches!(breaker.call(fail).await, Err(CallError::Failed(_))),
            "trial call must be attempted"
        );
        // Immediately re-opened: the very next call fails fast even though
        // only one failure happened since the trial.
        assert!(matches!(
            breaker.call(ok).await,
            Err(CallError::Open { .. })
        ));
    }
}
