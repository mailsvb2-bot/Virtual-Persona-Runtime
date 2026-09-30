    use super::*;
    use crate::{PersonaId, PersonaMode, PersonaVersion, VerifiedOwnerOpinion};

    fn profile_with_version(version: u64) -> PersonaProfile {
        PersonaProfile::new(
            PersonaIdentity::new(
                PersonaId::new("persona-1").unwrap(),
                PersonaVersion::new(version).unwrap(),
                PersonaMode::DigitalTwin,
            ),
            ConstitutionBoundary::strict_digital_twin(),
        )
    }

    fn profile() -> PersonaProfile {
        profile_with_version(1)
    }

    fn captured_opinion() -> OwnerClaimRecord {
        OwnerClaimRecord::capture(
            ClaimId::new("opinion-1").unwrap(),
            OwnerClaim {
                statement: "Мне нравится этот подход".into(),
                kind: ClaimKind::Opinion,
                source: SourceKind::Owner,
                verification: VerificationState::Unverified,
                derivation: DerivationKind::Direct,
            },
        )
        .unwrap()
    }

    #[test]
    fn captured_and_corrected_claims_enforce_canonical_size_limit() {
        let oversized = "Ж".repeat(MAX_OWNER_CLAIM_CHARS + 1);
        let capture = OwnerClaimRecord::capture(
            ClaimId::new("oversized").unwrap(),
            OwnerClaim {
                statement: oversized.clone(),
                kind: ClaimKind::Factual,
                source: SourceKind::Owner,
                verification: VerificationState::Unverified,
                derivation: DerivationKind::Direct,
            },
        );
        assert_eq!(capture, Err(ProfileError::ClaimStatementTooLong));

        let mut profile = profile();
        let id = ClaimId::new("opinion-1").unwrap();
        profile.add_captured_claim(captured_opinion()).unwrap();
        profile.mark_capture_complete().unwrap();
        assert_eq!(
            profile.correct_claim(&id, oversized, ClaimKind::Opinion),
            Err(ProfileError::ClaimStatementTooLong)
        );
    }

    #[test]
    fn captured_claim_requires_explicit_review_before_attribution() {
        let record = captured_opinion();
        assert!(VerifiedOwnerOpinion::try_from(record.current().claim().clone()).is_err());
    }

    #[test]
    fn initial_review_advances_persona_version_once() {
        let mut profile = profile();
        let id = ClaimId::new("opinion-1").unwrap();
        profile.add_captured_claim(captured_opinion()).unwrap();
        profile.mark_capture_complete().unwrap();
        profile.approve_claim(&id).unwrap();
        profile.approve_initial_review().unwrap();
        assert_eq!(profile.capture_state(), PersonaCaptureState::Reviewed);
        assert_eq!(profile.identity().version().get(), 2);
        let claim = profile.claim(&id).unwrap().current().claim().clone();
        assert!(VerifiedOwnerOpinion::try_from(claim).is_ok());
    }

    #[test]
    fn failed_transactional_correction_restores_exact_claim_history_and_version() {
        let mut profile = profile();
        let id = ClaimId::new("opinion-1").unwrap();
        profile.add_captured_claim(captured_opinion()).unwrap();
        profile.mark_capture_complete().unwrap();
        profile.approve_claim(&id).unwrap();
        profile.approve_initial_review().unwrap();

        let before_record = profile.claim(&id).unwrap().clone();
        let before_version = profile.identity().version();

        let result = profile.correct_claim_transactional(
            &id,
            "Не должен сохраниться",
            ClaimKind::Opinion,
            |_| Err::<(), _>("durable commit failed"),
        );

        assert_eq!(
            result,
            Err(TransactionalCorrectionError::Commit(
                "durable commit failed"
            ))
        );
        assert_eq!(profile.claim(&id).unwrap(), &before_record);
        assert_eq!(profile.identity().version(), before_version);
    }

    #[test]
    fn reviewed_correction_preserves_history_and_advances_persona_version() {
        let mut profile = profile();
        let id = ClaimId::new("opinion-1").unwrap();
        profile.add_captured_claim(captured_opinion()).unwrap();
        profile.mark_capture_complete().unwrap();
        profile.approve_claim(&id).unwrap();
        profile.approve_initial_review().unwrap();
        profile
            .correct_claim(
                &id,
                "Теперь я предпочитаю другой подход",
                ClaimKind::Opinion,
            )
            .unwrap();

        let record = profile.claim(&id).unwrap();
        assert_eq!(record.current().revision().get(), 3);
        assert_eq!(record.previous_revisions().len(), 2);
        assert_eq!(profile.identity().version().get(), 3);
        assert_eq!(
            record.current().claim().statement,
            "Теперь я предпочитаю другой подход"
        );
    }

    #[test]
    fn exact_retained_history_restores_without_replay() {
        let id = ClaimId::new("opinion-1").unwrap();
        let record = OwnerClaimRecord::restore_retained_history(
            id.clone(),
            vec![
                (
                    1,
                    OwnerClaim {
                        statement: "Исходное мнение".into(),
                        kind: ClaimKind::Opinion,
                        source: SourceKind::Owner,
                        verification: VerificationState::Unverified,
                        derivation: DerivationKind::Direct,
                    },
                ),
                (
                    2,
                    OwnerClaim {
                        statement: "Исходное мнение".into(),
                        kind: ClaimKind::Opinion,
                        source: SourceKind::Owner,
                        verification: VerificationState::OwnerVerified,
                        derivation: DerivationKind::Direct,
                    },
                ),
                (
                    3,
                    OwnerClaim {
                        statement: "Исправленное мнение".into(),
                        kind: ClaimKind::Preference,
                        source: SourceKind::Owner,
                        verification: VerificationState::OwnerVerified,
                        derivation: DerivationKind::Direct,
                    },
                ),
            ],
        )
        .unwrap();
        assert!(record.has_complete_history());
        let revisions: Vec<_> = record
            .retained_revisions()
            .map(|revision| {
                (
                    revision.revision().get(),
                    revision.claim().statement.clone(),
                    revision.claim().kind,
                    revision.claim().verification,
                )
            })
            .collect();
        assert_eq!(revisions[0].1, "Исходное мнение");
        assert_eq!(revisions[1].1, "Исходное мнение");
        assert_eq!(revisions[2].1, "Исправленное мнение");
        assert_eq!(revisions[2].2, ClaimKind::Preference);

        let restored = PersonaProfile::restore_reviewed(
            PersonaIdentity::new(
                PersonaId::new("persona-restored").unwrap(),
                PersonaVersion::new(3).unwrap(),
                PersonaMode::DigitalTwin,
            ),
            ConstitutionBoundary::strict_digital_twin(),
            vec![record],
        )
        .unwrap();
        assert_eq!(restored.capture_state(), PersonaCaptureState::Reviewed);
        assert_eq!(restored.claim(&id).unwrap().current().revision().get(), 3);
    }

    #[test]
    fn legacy_partial_history_remains_explicitly_partial() {
        let record = OwnerClaimRecord::restore_retained_history(
            ClaimId::new("legacy-claim").unwrap(),
            vec![(
                7,
                OwnerClaim {
                    statement: "Единственное известное legacy-значение".into(),
                    kind: ClaimKind::Factual,
                    source: SourceKind::Owner,
                    verification: VerificationState::OwnerVerified,
                    derivation: DerivationKind::Direct,
                },
            )],
        )
        .unwrap();
        assert!(!record.has_complete_history());
        assert!(record.previous_revisions().is_empty());
        assert_eq!(record.current().revision().get(), 7);
    }

    #[test]
    fn retained_history_allows_exhausted_current_revision() {
        let record = OwnerClaimRecord::restore_retained_history(
            ClaimId::new("exhausted-current").unwrap(),
            vec![(
                u64::MAX,
                OwnerClaim {
                    statement: "Последняя возможная ревизия".into(),
                    kind: ClaimKind::Factual,
                    source: SourceKind::Owner,
                    verification: VerificationState::OwnerVerified,
                    derivation: DerivationKind::Direct,
                },
            )],
        )
        .unwrap();
        assert_eq!(record.current().revision().get(), u64::MAX);
        assert!(!record.has_complete_history());
    }

    #[test]
    fn retained_history_rejects_gaps_and_fabricated_initial_verification() {
        let id = ClaimId::new("bad-history").unwrap();
        let verified = |statement: &str| OwnerClaim {
            statement: statement.into(),
            kind: ClaimKind::Opinion,
            source: SourceKind::Owner,
            verification: VerificationState::OwnerVerified,
            derivation: DerivationKind::Direct,
        };
        assert_eq!(
            OwnerClaimRecord::restore_retained_history(
                id.clone(),
                vec![(2, verified("a")), (4, verified("b"))]
            ),
            Err(ProfileError::InvalidClaimHistory)
        );
        assert_eq!(
            OwnerClaimRecord::restore_retained_history(id, vec![(1, verified("fabricated"))]),
            Err(ProfileError::InvalidClaimHistory)
        );
    }
    #[test]
    fn reviewed_correction_is_atomic_when_persona_version_is_exhausted() {
        let mut profile = profile_with_version(u64::MAX - 1);
        let id = ClaimId::new("opinion-1").unwrap();
        profile.add_captured_claim(captured_opinion()).unwrap();
        profile.mark_capture_complete().unwrap();
        profile.approve_claim(&id).unwrap();
        profile.approve_initial_review().unwrap();
        let before = profile.claim(&id).unwrap().current().clone();

        assert_eq!(
            profile.correct_claim(&id, "Новая позиция", ClaimKind::Opinion),
            Err(ProfileError::PersonaVersionExhausted)
        );
        assert_eq!(profile.identity().version().get(), u64::MAX);
        assert_eq!(profile.claim(&id).unwrap().current(), &before);
    }
