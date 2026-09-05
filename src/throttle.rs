use std::sync::Mutex;

/// Failed logins get progressively slower to answer.
///
/// The diary has exactly one account, so this is one counter rather than a map
/// keyed by address: an attacker cannot step around it by changing source IP,
/// and there is no per-client table to grow. The cost of that choice is that
/// someone hammering the login can slow the owner's own attempt down — which is
/// why the wait is capped at `MAX_DELAY` seconds instead of locking the account.
#[derive(Debug, Default)]
pub struct LoginThrottle {
    state: Mutex<State>,
}

#[derive(Debug, Default)]
struct State {
    failures: u32,
    next_allowed: i64,
}

/// Wrong passwords happen; the first few are answered at full speed.
const FREE_ATTEMPTS: u32 = 3;
const MAX_DELAY: i64 = 30;

/// 1s, 2s, 4s, 8s… up to `MAX_DELAY`, starting once the free attempts are used.
fn delay_after(failures: u32) -> i64 {
    match failures.checked_sub(FREE_ATTEMPTS) {
        None | Some(0) => 0,
        Some(over) => 1i64.checked_shl(over - 1).unwrap_or(MAX_DELAY).min(MAX_DELAY),
    }
}

impl LoginThrottle {
    /// `Err(seconds)` when this attempt is too soon after the last failure.
    pub fn check(&self) -> Result<(), i64> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        match state.next_allowed - crate::now() {
            wait if wait > 0 => Err(wait),
            _ => Ok(()),
        }
    }

    pub fn record_failure(&self) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.failures = state.failures.saturating_add(1);
        state.next_allowed = crate::now() + delay_after(state.failures);
    }

    pub fn record_success(&self) {
        *self.state.lock().unwrap_or_else(|e| e.into_inner()) = State::default();
    }
}

#[cfg(test)]
mod tests {
    use super::{delay_after, LoginThrottle, MAX_DELAY};

    #[test]
    fn the_first_few_mistakes_are_free() {
        assert_eq!(delay_after(0), 0);
        assert_eq!(delay_after(3), 0);
    }

    #[test]
    fn then_the_wait_doubles_up_to_the_cap() {
        assert_eq!(delay_after(4), 1);
        assert_eq!(delay_after(5), 2);
        assert_eq!(delay_after(6), 4);
        assert_eq!(delay_after(9), 30);
        // Never overflows, however long the attack runs.
        assert_eq!(delay_after(u32::MAX), MAX_DELAY);
    }

    #[test]
    fn a_correct_password_clears_the_debt() {
        let throttle = LoginThrottle::default();
        for _ in 0..10 {
            throttle.record_failure();
        }
        assert!(throttle.check().is_err());
        throttle.record_success();
        assert!(throttle.check().is_ok());
    }
}
