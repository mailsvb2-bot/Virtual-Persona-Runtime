use vpr_live_proof::{
    LiveProofPreflightError, LiveProofPreflightReceipt, ProviderConfigurationInspection,
    inspect_provider_configuration, inspect_provider_configuration_environment_only, preflight,
    preflight_environment_only,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CredentialMode {
    Auto,
    Environment,
}

impl CredentialMode {
    pub(crate) fn take_from_args(args: &mut Vec<String>) -> Self {
        if args
            .first()
            .is_some_and(|argument| argument == "--credentials=environment")
        {
            args.remove(0);
            Self::Environment
        } else {
            Self::Auto
        }
    }

    pub(crate) fn inspect(
        self,
        candidate_sha: &str,
        worktree_clean: bool,
    ) -> Result<ProviderConfigurationInspection, LiveProofPreflightError> {
        match self {
            Self::Auto => inspect_provider_configuration(candidate_sha, worktree_clean),
            Self::Environment => {
                inspect_provider_configuration_environment_only(candidate_sha, worktree_clean)
            }
        }
    }

    pub(crate) fn preflight(
        self,
        candidate_sha: &str,
        worktree_clean: bool,
        egress_authorized: bool,
    ) -> Result<LiveProofPreflightReceipt, LiveProofPreflightError> {
        match self {
            Self::Auto => preflight(candidate_sha, worktree_clean, egress_authorized),
            Self::Environment => {
                preflight_environment_only(candidate_sha, worktree_clean, egress_authorized)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::CredentialMode;

    #[test]
    fn environment_flag_is_consumed_only_when_leading() {
        let mut environment = vec!["--credentials=environment".to_owned(), "doctor".to_owned()];
        assert_eq!(
            CredentialMode::take_from_args(&mut environment),
            CredentialMode::Environment
        );
        assert_eq!(environment, ["doctor"]);

        let mut auto = vec!["doctor".to_owned(), "--credentials=environment".to_owned()];
        assert_eq!(
            CredentialMode::take_from_args(&mut auto),
            CredentialMode::Auto
        );
        assert_eq!(auto, ["doctor", "--credentials=environment"]);
    }
}
