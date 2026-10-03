#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum AvatarEnvironmentSelectionError {
    IncompleteOrAmbiguous,
}

pub(super) fn select_environment_avatar_provider_with(
    get: &mut impl FnMut(&'static str) -> Option<String>,
) -> Result<Option<String>, AvatarEnvironmentSelectionError> {
    if let Some(explicit) = get("VPR_OWNER_LAB_AVATAR_PROVIDER") {
        return Ok(Some(explicit.to_ascii_lowercase()));
    }

    let did_key = get("VPR_DID_API_KEY");
    let did_agent = get("VPR_DID_AGENT_ID");
    let local_endpoint = get("VPR_LOCAL_AVATAR_ENDPOINT");
    let local_token = get("VPR_LOCAL_AVATAR_API_TOKEN");
    let did_complete = did_key.is_some() && did_agent.is_some();
    let local_complete = local_endpoint.is_some() && local_token.is_some();
    let any_avatar_environment = did_key.is_some()
        || did_agent.is_some()
        || local_endpoint.is_some()
        || local_token.is_some();

    match (did_complete, local_complete, any_avatar_environment) {
        (true, false, _) => Ok(Some("did".into())),
        (false, true, _) => Ok(Some("local-open-source".into())),
        (false, false, false) => Ok(None),
        _ => Err(AvatarEnvironmentSelectionError::IncompleteOrAmbiguous),
    }
}

pub(super) fn avatar_provider_config_complete_with(
    get: &mut impl FnMut(&'static str) -> Option<String>,
) -> bool {
    let Ok(Some(selected)) = select_environment_avatar_provider_with(get) else {
        return false;
    };
    matches!(
        selected.as_str(),
        "did" | "d-id" | "did-agent-streams" | "local" | "local-open-source"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_environment_is_selected_without_did() {
        let values = [
            ("VPR_LOCAL_AVATAR_ENDPOINT", "https://avatar.example.test"),
            ("VPR_LOCAL_AVATAR_API_TOKEN", "worker-token"),
        ];
        let mut get = |name| {
            values
                .iter()
                .find_map(|(key, value)| (*key == name).then(|| (*value).to_owned()))
        };
        assert_eq!(
            select_environment_avatar_provider_with(&mut get),
            Ok(Some("local-open-source".into()))
        );
    }

    #[test]
    fn both_complete_avatar_environments_are_ambiguous_without_selector() {
        let values = [
            "VPR_DID_API_KEY",
            "VPR_DID_AGENT_ID",
            "VPR_LOCAL_AVATAR_ENDPOINT",
            "VPR_LOCAL_AVATAR_API_TOKEN",
        ];
        let mut get =
            |name| values.contains(&name).then(|| "configured".to_owned());
        assert_eq!(
            select_environment_avatar_provider_with(&mut get),
            Err(AvatarEnvironmentSelectionError::IncompleteOrAmbiguous)
        );
    }

    #[test]
    fn partial_avatar_environment_fails_closed_instead_of_falling_back() {
        let mut get = |name| {
            (name == "VPR_LOCAL_AVATAR_ENDPOINT").then(|| "https://avatar.example.test".to_owned())
        };
        assert_eq!(
            select_environment_avatar_provider_with(&mut get),
            Err(AvatarEnvironmentSelectionError::IncompleteOrAmbiguous)
        );
    }
}
