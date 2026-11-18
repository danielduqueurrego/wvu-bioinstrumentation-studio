//! One owner at a time for the Arduino serial port and the acquisition session.
//!
//! Tauri commands that touch the board run on worker threads, so the main
//! thread no longer serializes them by accident. Every operation that opens
//! the COM port or changes the shared session state (a recording, a firmware
//! restore, Verify Firmware, Retry connection) first claims this lease. The
//! claim is atomic, a conflicting claim is refused with a student-readable
//! busy message, and the lease is released by `Drop` on every exit path,
//! including an error return or a panic that unwinds the owning thread.
use serde::Serialize;
use std::{
    fmt,
    sync::{Arc, Condvar, Mutex, PoisonError},
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SerialOwner {
    /// Nothing owns the port or the session.
    #[default]
    Idle,
    /// A recording (simulator or hardware), from Start until its worker has
    /// finished saving.
    Session,
    /// A firmware restore job, from Restore until its worker has finished.
    FirmwareJob,
    /// A read-only Verify Firmware handshake.
    Verify,
    /// A Retry connection handshake.
    Retry,
    /// Repair Arduino tools. It replaces the tools that Start, Verify,
    /// Retry, and Restore run, so none of them may run during a repair.
    ToolRepair,
}

impl SerialOwner {
    /// What a student is told when this owner blocks another operation.
    pub fn busy_message(self) -> &'static str {
        match self {
            Self::Session => {
                "The Arduino is busy with a recording. Stop it and wait until it is saved, then try again."
            }
            Self::FirmwareJob => {
                "WVU Firmware is being restored. Wait for the restore to finish, then try again."
            }
            Self::Verify => {
                "The firmware is being verified. Wait for Verify Firmware to finish, then try again."
            }
            Self::Retry => {
                "The connection is being retried. Wait for Retry connection to finish, then try again."
            }
            Self::ToolRepair => {
                "The Arduino tools are being repaired. Wait for the repair to finish, then try again."
            }
            Self::Idle => {
                "The Arduino is busy. Wait for the current operation to finish, then try again."
            }
        }
    }
}

/// A claim was refused because `holder` owns the port and session.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SerialBusy {
    pub requested: SerialOwner,
    pub holder: SerialOwner,
}

impl SerialBusy {
    pub fn message(&self) -> &'static str {
        self.holder.busy_message()
    }
}

impl fmt::Display for SerialBusy {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.message())
    }
}

impl std::error::Error for SerialBusy {}

#[derive(Default)]
struct OwnershipState {
    owner: Mutex<SerialOwner>,
    released: Condvar,
}

/// The shared owner state. Clones refer to the same owner.
#[derive(Clone, Default)]
pub struct SerialOwnership {
    shared: Arc<OwnershipState>,
}

impl SerialOwnership {
    pub fn current(&self) -> SerialOwner {
        // The guarded value is a plain enum that is never left half-written,
        // so a poisoned lock still holds a valid owner.
        *self
            .shared
            .owner
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// Claims the lease now or reports who holds it.
    pub fn try_claim(&self, owner: SerialOwner) -> Result<SerialLease, SerialBusy> {
        self.claim_within(owner, Duration::ZERO)
    }

    /// Claims the lease, waiting at most `grace` for the current owner to
    /// release it. The short grace covers a worker that has already published
    /// its final state and is returning; a real conflict is still refused.
    pub fn claim_within(
        &self,
        owner: SerialOwner,
        grace: Duration,
    ) -> Result<SerialLease, SerialBusy> {
        let deadline = Instant::now() + grace;
        let mut current = self
            .shared
            .owner
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if owner == SerialOwner::Idle {
            return Err(SerialBusy {
                requested: owner,
                holder: *current,
            });
        }
        while *current != SerialOwner::Idle {
            let now = Instant::now();
            if now >= deadline {
                return Err(SerialBusy {
                    requested: owner,
                    holder: *current,
                });
            }
            current = self
                .shared
                .released
                .wait_timeout(current, deadline - now)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
        *current = owner;
        Ok(SerialLease {
            shared: Arc::clone(&self.shared),
            owner,
        })
    }
}

/// Proof that the holder owns the port and session. Dropping it releases them.
pub struct SerialLease {
    shared: Arc<OwnershipState>,
    owner: SerialOwner,
}

impl SerialLease {
    pub fn owner(&self) -> SerialOwner {
        self.owner
    }
}

impl fmt::Debug for SerialLease {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SerialLease")
            .field("owner", &self.owner)
            .finish()
    }
}

impl Drop for SerialLease {
    fn drop(&mut self) {
        {
            let mut current = self
                .shared
                .owner
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            if *current == self.owner {
                *current = SerialOwner::Idle;
            }
        }
        self.shared.released.notify_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;

    #[test]
    fn a_second_owner_is_refused_until_the_lease_is_dropped() {
        let ownership = SerialOwnership::default();
        assert_eq!(ownership.current(), SerialOwner::Idle);
        let lease = ownership
            .try_claim(SerialOwner::FirmwareJob)
            .unwrap_or_else(|busy| panic!("{busy}"));
        assert_eq!(lease.owner(), SerialOwner::FirmwareJob);
        assert_eq!(ownership.current(), SerialOwner::FirmwareJob);
        for requested in [
            SerialOwner::Session,
            SerialOwner::Verify,
            SerialOwner::Retry,
            SerialOwner::FirmwareJob,
        ] {
            let busy = ownership
                .try_claim(requested)
                .err()
                .unwrap_or_else(|| panic!("{requested:?} must be refused"));
            assert_eq!(busy.requested, requested);
            assert_eq!(busy.holder, SerialOwner::FirmwareJob);
            assert!(busy.to_string().contains("being restored"), "{busy}");
        }
        drop(lease);
        assert_eq!(ownership.current(), SerialOwner::Idle);
        let verify = ownership
            .try_claim(SerialOwner::Verify)
            .unwrap_or_else(|busy| panic!("{busy}"));
        assert_eq!(ownership.current(), verify.owner());
    }

    #[test]
    fn idle_is_not_an_owner_that_can_be_claimed() {
        let ownership = SerialOwnership::default();
        assert!(ownership.try_claim(SerialOwner::Idle).is_err());
        assert_eq!(ownership.current(), SerialOwner::Idle);
    }

    #[test]
    fn a_panicking_owner_still_releases_the_lease() {
        let ownership = SerialOwnership::default();
        let lease = ownership
            .try_claim(SerialOwner::Session)
            .unwrap_or_else(|busy| panic!("{busy}"));
        let worker = thread::spawn(move || {
            let _lease = lease;
            panic!("simulated worker failure");
        });
        assert!(worker.join().is_err());
        assert_eq!(ownership.current(), SerialOwner::Idle);
        assert!(ownership.try_claim(SerialOwner::Retry).is_ok());
    }

    #[test]
    fn a_grace_period_waits_for_an_owner_that_is_finishing() {
        let ownership = SerialOwnership::default();
        let lease = ownership
            .try_claim(SerialOwner::Session)
            .unwrap_or_else(|busy| panic!("{busy}"));
        let finishing = thread::spawn(move || {
            thread::sleep(Duration::from_millis(30));
            drop(lease);
        });
        let claimed = ownership
            .claim_within(SerialOwner::Verify, Duration::from_secs(5))
            .unwrap_or_else(|busy| panic!("{busy}"));
        assert_eq!(claimed.owner(), SerialOwner::Verify);
        finishing
            .join()
            .unwrap_or_else(|_| panic!("finishing thread panicked"));
    }

    #[test]
    fn a_grace_period_does_not_wait_forever_for_a_real_conflict() {
        let ownership = SerialOwnership::default();
        let _job = ownership
            .try_claim(SerialOwner::FirmwareJob)
            .unwrap_or_else(|busy| panic!("{busy}"));
        let started = Instant::now();
        let busy = ownership
            .claim_within(SerialOwner::Session, Duration::from_millis(40))
            .err()
            .unwrap_or_else(|| panic!("a running job must refuse a recording"));
        assert!(started.elapsed() >= Duration::from_millis(40));
        assert_eq!(busy.holder, SerialOwner::FirmwareJob);
    }

    #[test]
    fn busy_messages_are_plain_sentences() {
        for holder in [
            SerialOwner::Idle,
            SerialOwner::Session,
            SerialOwner::FirmwareJob,
            SerialOwner::Verify,
            SerialOwner::Retry,
            SerialOwner::ToolRepair,
        ] {
            let message = holder.busy_message();
            assert!(message.ends_with('.'), "{message}");
            assert!(!message.contains('_'), "{message}");
        }
    }
}
