use crate::PersonaId;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PersonaMode {
    DigitalTwin,
    FictionalCharacter,
    Expert,
    OrganizationRepresentative,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct PersonaVersion(u64);

impl PersonaVersion {
    #[must_use]
    pub fn new(value: u64) -> Option<Self> {
        (value > 0).then_some(Self(value))
    }

    #[must_use]
    pub fn get(self) -> u64 {
        self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersonaIdentity {
    id: PersonaId,
    version: PersonaVersion,
    mode: PersonaMode,
}

impl PersonaIdentity {
    #[must_use]
    pub const fn new(id: PersonaId, version: PersonaVersion, mode: PersonaMode) -> Self {
        Self { id, version, mode }
    }

    #[must_use]
    pub fn id(&self) -> &PersonaId {
        &self.id
    }

    #[must_use]
    pub const fn version(&self) -> PersonaVersion {
        self.version
    }

    #[must_use]
    pub const fn mode(&self) -> PersonaMode {
        self.mode
    }

    #[must_use]
    pub fn is_rt0_supported(&self) -> bool {
        self.mode == PersonaMode::DigitalTwin
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConstitutionBoundary {
    pub may_represent_owner: bool,
    pub inferred_views_must_be_labeled: bool,
}

impl ConstitutionBoundary {
    #[must_use]
    pub const fn strict_digital_twin() -> Self {
        Self {
            may_represent_owner: true,
            inferred_views_must_be_labeled: true,
        }
    }
}
