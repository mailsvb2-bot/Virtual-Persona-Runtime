use std::{fs, path::Path};

use vpr_evaluation::Rt0ExitSupportingArtifacts;

pub(super) struct SupportingArtifactBytes {
    ci: Vec<u8>,
    e2e: Vec<u8>,
    owner_conversation: Vec<u8>,
    visitor_conversation: Vec<u8>,
    acceptance: Vec<u8>,
    quality: Vec<u8>,
    cost: Vec<u8>,
    privacy_permissions: Vec<u8>,
    human_evaluation: Vec<u8>,
    known_limitations: Vec<u8>,
}

impl SupportingArtifactBytes {
    pub(super) fn read(root: &Path) -> Option<Self> {
        Some(Self {
            ci: fs::read(root.join("ci-evidence.json")).ok()?,
            e2e: fs::read(root.join("e2e-evidence.json")).ok()?,
            owner_conversation: fs::read(root.join("owner-conversation.json")).ok()?,
            visitor_conversation: fs::read(root.join("visitor-conversation.json")).ok()?,
            acceptance: fs::read(root.join("acceptance.json")).ok()?,
            quality: fs::read(root.join("quality.json")).ok()?,
            cost: fs::read(root.join("cost.json")).ok()?,
            privacy_permissions: fs::read(root.join("privacy-permissions.json")).ok()?,
            human_evaluation: fs::read(root.join("human-evaluation.json")).ok()?,
            known_limitations: fs::read(root.join("known-limitations.md")).ok()?,
        })
    }

    pub(super) fn as_verification(&self) -> Rt0ExitSupportingArtifacts<'_> {
        Rt0ExitSupportingArtifacts {
            ci: &self.ci,
            e2e: &self.e2e,
            owner_conversation: &self.owner_conversation,
            visitor_conversation: &self.visitor_conversation,
            acceptance: &self.acceptance,
            quality: &self.quality,
            cost: &self.cost,
            privacy_permissions: &self.privacy_permissions,
            human_evaluation: &self.human_evaluation,
            known_limitations: &self.known_limitations,
        }
    }
}
