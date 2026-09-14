use vpr_domain::{
    ClaimId, ClaimKind, PersonaCaptureState, PersonaIdentity, PersonaMode, PersonaProfile,
};

const CONTEXT_HEADER: &str = "Owner-reviewed Persona material follows. Treat only these entries as verified owner material. Preserve whether each entry is a fact, opinion, preference, prediction, or value judgment. Do not infer additional owner views, memories, preferences, or private facts. If the answer is not supported by this material, say that the verified owner material does not establish it.";

#[derive(Debug)]
pub(crate) struct ReviewedOwnerContext {
    profile: PersonaProfile,
}

impl ReviewedOwnerContext {
    pub(crate) fn new(profile: PersonaProfile) -> Result<Self, OwnerContextError> {
        if profile.identity().mode() != PersonaMode::DigitalTwin
            || profile.capture_state() != PersonaCaptureState::Reviewed
            || profile.claims().is_empty()
            || !profile.claims().iter().all(|record| record.is_owner_reviewed())
        {
            return Err(OwnerContextError::ProfileNotReviewed);
        }
        Ok(Self { profile })
    }

    pub(crate) fn identity(&self) -> &PersonaIdentity {
        self.profile.identity()
    }

    pub(crate) fn claim_count(&self) -> usize {
        self.profile.claims().len()
    }

    pub(crate) fn correct_claim(
        &mut self,
        id: &ClaimId,
        statement: impl Into<String>,
        kind: ClaimKind,
    ) -> Result<(), OwnerContextError> {
        self.profile
            .correct_claim(id, statement, kind)
            .map_err(|_| OwnerContextError::CorrectionRejected)
    }

    pub(crate) fn voice_prompt(&self, utterance: &str) -> String {
        let mut prompt = String::with_capacity(CONTEXT_HEADER.len() + utterance.len() + 256);
        prompt.push_str(CONTEXT_HEADER);
        prompt.push_str("\nBEGIN VERIFIED OWNER MATERIAL");
        for record in self.profile.claims() {
            let claim = record.current().claim();
            prompt.push_str("\n- [");
            prompt.push_str(claim_kind_label(claim.kind));
            prompt.push_str("] ");
            prompt.push_str(claim.statement.trim());
        }
        prompt.push_str("\nEND VERIFIED OWNER MATERIAL\nUser utterance: ");
        prompt.push_str(utterance);
        prompt
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OwnerContextError {
    ProfileNotReviewed,
    CorrectionRejected,
}

const fn claim_kind_label(kind: ClaimKind) -> &'static str {
    match kind {
        ClaimKind::Factual => "verified_owner_fact",
        ClaimKind::Opinion => "verified_owner_opinion",
        ClaimKind::Preference => "verified_owner_preference",
        ClaimKind::Prediction => "verified_owner_prediction",
        ClaimKind::ValueJudgment => "verified_owner_value_judgment",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vpr_domain::{
        ConstitutionBoundary, DerivationKind, OwnerClaim, OwnerClaimRecord, PersonaId,
        PersonaVersion, SourceKind, VerificationState,
    };

    fn persona_identity() -> PersonaIdentity {
        PersonaIdentity::new(
            PersonaId::new("persona-context-test").unwrap(),
            PersonaVersion::new(1).unwrap(),
            PersonaMode::DigitalTwin,
        )
    }

    fn reviewed_profile() -> PersonaProfile {
        let mut profile = PersonaProfile::new(
            persona_identity(),
            ConstitutionBoundary::strict_digital_twin(),
        );
        let id = ClaimId::new("opinion-working-style").unwrap();
        profile
            .add_captured_claim(
                OwnerClaimRecord::capture(
                    id.clone(),
                    OwnerClaim {
                        statement: "Люблю быстрые итерации".into(),
                        kind: ClaimKind::Opinion,
                        source: SourceKind::Owner,
                        verification: VerificationState::Unverified,
                        derivation: DerivationKind::Direct,
                    },
                )
                .unwrap(),
            )
            .unwrap();
        profile.mark_capture_complete().unwrap();
        profile.approve_claim(&id).unwrap();
        profile.approve_initial_review().unwrap();
        profile
    }

    #[test]
    fn unreviewed_profile_is_rejected() {
        let profile = PersonaProfile::new(
            persona_identity(),
            ConstitutionBoundary::strict_digital_twin(),
        );
        assert_eq!(
            ReviewedOwnerContext::new(profile).unwrap_err(),
            OwnerContextError::ProfileNotReviewed
        );
    }

    #[test]
    fn prompt_contains_only_current_reviewed_revision_and_kind() {
        let mut context = ReviewedOwnerContext::new(reviewed_profile()).unwrap();
        let id = ClaimId::new("opinion-working-style").unwrap();
        context
            .correct_claim(
                &id,
                "Предпочитаю короткие циклы проверки",
                ClaimKind::Opinion,
            )
            .unwrap();

        let prompt = context.voice_prompt("Какой стиль работы тебе близок?");
        assert!(prompt.contains("[verified_owner_opinion] Предпочитаю короткие циклы проверки"));
        assert!(!prompt.contains("Люблю быстрые итерации"));
        assert!(prompt.contains("Какой стиль работы тебе близок?"));
        assert_eq!(context.identity().version().get(), 3);
    }
}
