use vpr_integration::{CancellationProbe, ProviderError, ProviderErrorKind};
use vpr_policy::{AuthorityScope, DataClass};

use crate::cancellation::ProviderCancellation;
use crate::error::{ProviderExecutionError, RuntimeDenyReason};
use crate::execution_gate::{ProviderEgressGuard, SessionExecutionGate};

/// Internal linearized capability for one immediate provider operation.
///
/// It is never exposed to callers; public provider execution methods acquire and consume it
/// inside one runtime call, eliminating delayed or repeated permit use.
#[derive(Debug)]
pub(crate) struct ProviderExecutionPermit {
    pub(crate) cancellation: ProviderCancellation,
    pub(crate) gate: SessionExecutionGate,
}

impl ProviderExecutionPermit {
    /// The provider can ignore cancellation and return a late success. Never
    /// promote that result into a runtime output or a new stream capability.
    /// This postflight is shared by owner and visitor turns; it is NOT a
    /// substitute for cancellation-aware transport dispatch or remote STOP.
    /// Common RT0/RT1 dispatch boundary. Refuse an operation revoked
    /// between permit issuance and provider invocation. The transport must
    /// still abort in-flight I/O and independently confirm remote STOP.
    pub(crate) fn dispatch<T>(
        &self,
        operation: impl FnOnce(&dyn CancellationProbe) -> Result<T, ProviderError>,
    ) -> Result<T, ProviderExecutionError> {
        let _in_flight = self
            .gate
            .begin_provider_egress(&self.cancellation)
            .map_err(|()| ProviderExecutionError::Denied(RuntimeDenyReason::TurnCancelled))?;
        // No session/authorization lock is held during external network I/O.
        // A concurrent revoke cancels this admitted transport; its completion
        // remains tracked until the provider call and postflight have returned.
        self.finish(operation(&self.cancellation))
    }

    /// Keep a pull-based LLM/STT stream counted as active even between
    /// chunks, until its provider transport is dropped. Without this guard a
    /// paused owner could falsely observe quiescence between stream polls.
    pub(crate) fn track_stream_lifetime(
        &self,
    ) -> Result<ProviderEgressGuard, ProviderExecutionError> {
        self.gate
            .begin_provider_egress(&self.cancellation)
            .map_err(|()| ProviderExecutionError::Denied(RuntimeDenyReason::TurnCancelled))
    }

    pub(crate) fn finish<T>(
        &self,
        result: Result<T, ProviderError>,
    ) -> Result<T, ProviderExecutionError> {
        if self.cancellation.is_cancelled() {
            // Preserve the existing typed provider-cancellation contract.
            // Only a late success (or unrelated late failure) is overridden.
            return match result {
                Err(error) if error.kind == ProviderErrorKind::Cancelled => {
                    Err(ProviderExecutionError::Provider(error))
                }
                _ => Err(ProviderExecutionError::Denied(
                    RuntimeDenyReason::TurnCancelled,
                )),
            };
        }
        result.map_err(ProviderExecutionError::from)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProviderOperation {
    Llm,
    Stt,
    Tts,
    Avatar,
}

impl ProviderOperation {
    /// RT0 fail-closed rule: until payload provenance can prove a lower class,
    /// every unstructured external-provider payload is treated as biometric.
    #[must_use]
    pub(crate) const fn data_class(self) -> DataClass {
        match self {
            Self::Llm | Self::Stt | Self::Tts | Self::Avatar => DataClass::Biometric,
        }
    }

    #[must_use]
    pub(crate) fn required_scope(self) -> AuthorityScope {
        let scope = match self {
            Self::Llm | Self::Stt | Self::Tts | Self::Avatar => "provider.egress",
        };
        AuthorityScope::new(scope).expect("canonical provider scope is non-empty")
    }
}

#[cfg(test)]
mod dispatch_tests {
    use std::cell::Cell;
    use std::sync::Arc;

    use super::*;
    use crate::cancellation::TurnCancellation;
    use crate::clock::SystemClock;

    fn permit(turn: TurnCancellation) -> ProviderExecutionPermit {
        ProviderExecutionPermit {
            cancellation: ProviderCancellation::new(turn, Arc::new(SystemClock), None),
            gate: SessionExecutionGate::default(),
        }
    }

    #[test]
    fn cancelled_between_permit_and_dispatch_never_calls_provider() {
        let turn = TurnCancellation::default();
        let permit = permit(turn.clone());
        turn.cancel();
        let calls = Cell::new(0);
        let result: Result<(), _> = permit.dispatch(|_| {
            calls.set(calls.get() + 1);
            Ok(())
        });
        assert_eq!(calls.get(), 0);
        assert_eq!(
            result,
            Err(ProviderExecutionError::Denied(
                RuntimeDenyReason::TurnCancelled
            ))
        );
    }

    #[test]
    fn cancellation_during_provider_call_overrides_late_success() {
        let turn = TurnCancellation::default();
        let permit = permit(turn.clone());
        let result = permit.dispatch(|_| {
            turn.cancel();
            Ok(42)
        });
        assert_eq!(
            result,
            Err(ProviderExecutionError::Denied(
                RuntimeDenyReason::TurnCancelled
            ))
        );
    }

    #[test]
    fn cancelled_provider_error_keeps_existing_typed_contract() {
        let turn = TurnCancellation::default();
        let permit = permit(turn.clone());
        let result: Result<(), _> = permit.dispatch(|_| {
            turn.cancel();
            Err(ProviderError {
                kind: ProviderErrorKind::Cancelled,
                retryable: false,
            })
        });
        assert_eq!(
            result,
            Err(ProviderExecutionError::Provider(ProviderError {
                kind: ProviderErrorKind::Cancelled,
                retryable: false,
            }))
        );
    }

    #[test]
    fn revoke_linearizes_with_dispatch_admission_without_holding_network_lock() {
        use std::sync::mpsc;
        use std::thread;

        let turn = TurnCancellation::default();
        let gate = SessionExecutionGate::default();
        let permit = ProviderExecutionPermit {
            cancellation: ProviderCancellation::new(turn.clone(), Arc::new(SystemClock), None),
            gate: gate.clone(),
        };
        let writer = gate.write().expect("authority transaction");
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let worker = thread::spawn(move || {
            ready_tx.send(()).expect("notify");
            let called = Cell::new(false);
            let result: Result<(), _> = permit.dispatch(|_| {
                called.set(true);
                Ok(())
            });
            (result, called.get())
        });
        ready_rx.recv().expect("worker started");
        // This cancellation is serialized with the dispatch read lock.
        turn.cancel();
        drop(writer);
        let (result, called) = worker.join().expect("dispatch");
        assert!(!called, "revoked authority must never start provider I/O");
        assert_eq!(
            result,
            Err(ProviderExecutionError::Denied(
                RuntimeDenyReason::TurnCancelled
            ))
        );
        assert!(gate.await_provider_quiescence(std::time::Duration::ZERO));
    }

    #[test]
    fn revoke_does_not_wait_for_stalled_provider_but_quiescence_is_bounded() {
        use std::sync::mpsc;
        use std::thread;
        use std::time::Duration;

        let turn = TurnCancellation::default();
        let gate = SessionExecutionGate::default();
        let permit = ProviderExecutionPermit {
            cancellation: ProviderCancellation::new(turn.clone(), Arc::new(SystemClock), None),
            gate: gate.clone(),
        };
        let (entered_tx, entered_rx) = mpsc::sync_channel(1);
        let (release_tx, release_rx) = mpsc::sync_channel(1);
        let worker = thread::spawn(move || {
            permit.dispatch(|cancellation| {
                entered_tx.send(()).expect("provider entered");
                release_rx.recv().expect("provider released");
                assert!(cancellation.is_cancelled());
                Ok(42)
            })
        });
        entered_rx.recv().expect("provider started");
        // Authority revocation can acquire its write lock while the provider
        // is stalled: the network operation does not hold that lock.
        let writer = gate.write().expect("revoke must not wait for provider");
        turn.cancel();
        drop(writer);
        assert!(
            !gate.await_provider_quiescence(Duration::from_millis(10)),
            "an outstanding provider must not be reported as stopped"
        );
        release_tx.send(()).expect("release");
        assert_eq!(
            worker.join().expect("provider completion"),
            Err(ProviderExecutionError::Denied(
                RuntimeDenyReason::TurnCancelled
            ))
        );
        assert!(gate.await_provider_quiescence(Duration::from_millis(10)));
    }

    #[test]
    fn expired_permit_cannot_dispatch_provider_operation() {
        let permit = ProviderExecutionPermit {
            cancellation: ProviderCancellation::new(
                TurnCancellation::default(),
                Arc::new(SystemClock),
                Some(0),
            ),
            gate: SessionExecutionGate::default(),
        };
        let called = Cell::new(false);
        let result: Result<(), _> = permit.dispatch(|_| {
            called.set(true);
            Ok(())
        });
        assert!(!called.get());
        assert_eq!(
            result,
            Err(ProviderExecutionError::Denied(
                RuntimeDenyReason::TurnCancelled
            ))
        );
    }
}
