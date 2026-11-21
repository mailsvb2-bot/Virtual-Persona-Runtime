//! Transient, bearer-authorized interview sessions for new owner enrollment.
//! Only the ONE canonical Persona envelope persists completed owner accounts.
//! An unfinished capture is never an owner, publication or provider authority.
use std::collections::HashMap;
use std::fmt::Write as _;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use sha2::{Digest, Sha256};
use vpr_domain::{ClaimId, PersonaId};

use crate::owner_capture::{OwnerCaptureSnapshot, Rt0OwnerCapture};
use crate::owner_context::durable_snapshot_from_reviewed_profile;
use crate::rt1_workspace_service::Rt1WorkspaceService;

const MAX_PENDING: usize = 32;
const PENDING_LIFETIME: Duration = Duration::from_secs(60 * 60);

struct PendingEnrollment {
    capture: Rt0OwnerCapture,
    created: Instant,
}

/// Pending onboarding is transient, capacity bounded, and has no account
/// credentials. Public authority is created only by the canonical commit.
#[derive(Default)]
pub(crate) struct Rt1OwnerEnrollments {
    pending: Mutex<HashMap<String, PendingEnrollment>>,
}

impl Rt1OwnerEnrollments {
    fn with_capture<T>(
        &self,
        enrollment_token: &str,
        action: impl FnOnce(&mut Rt0OwnerCapture) -> Result<T, String>,
    ) -> Result<T, String> {
        if enrollment_token.len() != 64
            || !enrollment_token.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            return Err("RT1_ENROLLMENT_DENIED".into());
        }
        let digest = enrollment_digest(enrollment_token);
        let mut pending = self.pending.lock();
        pending.retain(|_, value| value.created.elapsed() < PENDING_LIFETIME);
        let session = pending.get_mut(&digest).ok_or("RT1_ENROLLMENT_DENIED")?;
        action(&mut session.capture)
    }

    pub(crate) fn begin(
        &self,
        persona_id: String,
    ) -> Result<(String, OwnerCaptureSnapshot), String> {
        let identity = PersonaId::new(persona_id).map_err(|_| "INVALID_PERSONA_ID")?;
        let capture = Rt0OwnerCapture::new(identity).map_err(|_| "RT1_CAPTURE_UNAVAILABLE")?;
        let mut bytes = [0_u8; 32];
        getrandom::fill(&mut bytes).map_err(|_| "RT1_ENROLLMENT_ENTROPY_UNAVAILABLE")?;
        let mut token = String::with_capacity(64);
        for byte in bytes {
            write!(&mut token, "{byte:02x}").map_err(|_| "RT1_ENROLLMENT_ENTROPY_UNAVAILABLE")?;
        }
        let digest = enrollment_digest(&token);
        let snapshot = capture.snapshot();
        let mut pending = self.pending.lock();
        pending.retain(|_, value| value.created.elapsed() < PENDING_LIFETIME);
        if pending.len() >= MAX_PENDING || pending.contains_key(&digest) {
            return Err("RT1_ENROLLMENT_CAPACITY_EXCEEDED".into());
        }
        pending.insert(
            digest,
            PendingEnrollment {
                capture,
                created: Instant::now(),
            },
        );
        Ok((token, snapshot))
    }

    pub(crate) fn snapshot(&self, token: &str) -> Result<OwnerCaptureSnapshot, String> {
        self.with_capture(token, |capture| Ok(capture.snapshot()))
    }

    pub(crate) fn answer(
        &self,
        token: &str,
        answer: String,
    ) -> Result<OwnerCaptureSnapshot, String> {
        self.with_capture(token, |capture| {
            capture
                .submit_answer(answer)
                .map_err(|_| "RT1_CAPTURE_ANSWER_DENIED")?;
            Ok(capture.snapshot())
        })
    }

    pub(crate) fn finish(&self, token: &str) -> Result<OwnerCaptureSnapshot, String> {
        self.with_capture(token, |capture| {
            capture
                .finish_capture()
                .map_err(|_| "RT1_CAPTURE_FINISH_DENIED")?;
            Ok(capture.snapshot())
        })
    }

    pub(crate) fn approve(
        &self,
        token: &str,
        claim_id: &str,
    ) -> Result<OwnerCaptureSnapshot, String> {
        let id = ClaimId::new(claim_id.to_owned()).map_err(|_| "INVALID_CLAIM_ID")?;
        self.with_capture(token, |capture| {
            capture
                .approve_claim(&id)
                .map_err(|_| "RT1_CAPTURE_REVIEW_DENIED")?;
            Ok(capture.snapshot())
        })
    }

    /// An account token exists only after an atomic commit of owner-reviewed
    /// Persona material in the existing canonical store. Failed commits retain
    /// the pending review so an owner may retry without starting again.
    pub(crate) fn complete(
        &self,
        token: &str,
        workspace: &Rt1WorkspaceService,
    ) -> Result<(String, String), String> {
        self.with_capture(token, |capture| {
            if capture.snapshot().capture_state != "reviewed" {
                capture
                    .complete_initial_review()
                    .map_err(|_| "RT1_CAPTURE_REVIEW_INCOMPLETE")?;
            }
            let reviewed = durable_snapshot_from_reviewed_profile(capture.profile())
                .map_err(|_| "RT1_CAPTURE_REVIEW_INCOMPLETE")?;
            workspace.provision_verified_additional_owner(reviewed)
        })
        .map(|account| {
            self.pending.lock().remove(&enrollment_digest(token));
            account
        })
    }
}

fn enrollment_digest(token: &str) -> String {
    format!("{:x}", Sha256::digest(token.as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parallel_owner_interviews_require_their_own_random_bearer() {
        let enrollments = Rt1OwnerEnrollments::default();
        let (a_token, a_start) = enrollments.begin("enrolling-owner-a".into()).unwrap();
        let (b_token, b_start) = enrollments.begin("enrolling-owner-b".into()).unwrap();
        assert_eq!(a_token.len(), 64);
        assert_eq!(b_token.len(), 64);
        assert_ne!(a_token, b_token);
        assert_ne!(a_start.persona_id, b_start.persona_id);
        assert!(enrollments.snapshot(&"0".repeat(64)).is_err());
        assert!(enrollments.snapshot("bad").is_err());
        let a_answered = enrollments
            .answer(&a_token, "Owner A's personally reviewed answer".into())
            .unwrap();
        assert_eq!(a_answered.answered_count, 1);
        let b_status = enrollments.snapshot(&b_token).unwrap();
        assert_eq!(b_status.answered_count, 0);
        assert_eq!(
            enrollments.snapshot(&a_token).unwrap().answered_count,
            1
        );
        assert!(enrollments.finish(&b_token).is_err());
        assert!(enrollments.approve(&b_token, "unknown-claim").is_err());
    }

    #[test]
    fn registration_preserves_real_capture_review_gate() {
        let enrollments = Rt1OwnerEnrollments::default();
        let (token, _) = enrollments.begin("reviewed-new-owner".into()).unwrap();
        for _ in 0..20 {
            let snapshot = enrollments.snapshot(&token).unwrap();
            let Some(question) = snapshot.current_question else {
                break;
            };
            enrollments
                .answer(
                    &token,
                    format!("Owner-reviewed material for {}", question.claim_id),
                )
                .unwrap();
        }
        let finished = enrollments.finish(&token).unwrap();
        assert!(!finished.claims.is_empty());
        assert!(enrollments.with_capture(&token, |capture| {
            capture.complete_initial_review().map_err(|_| "not reviewed".to_owned())
        }).is_err());
        for claim in finished.claims {
            enrollments.approve(&token, &claim.claim_id).unwrap();
        }
        enrollments
            .with_capture(&token, |capture| {
                capture
                    .complete_initial_review()
                    .map_err(|_| "review incomplete".to_owned())?;
                let durable = durable_snapshot_from_reviewed_profile(capture.profile())
                    .map_err(|_| "not reviewed".to_owned())?;
                assert_eq!(durable.persona_id, "reviewed-new-owner");
                assert!(durable.persona_version >= 2);
                Ok(())
            })
            .unwrap();
        assert!(enrollments.snapshot(&"f".repeat(64)).is_err());
    }
}
