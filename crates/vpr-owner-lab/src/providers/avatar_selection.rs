pub(super) fn select_avatar_provider_with(
    get: &mut impl FnMut(&'static str) -> Option<String>,
) -> Option<String> {
    if let Some(explicit) = get("VPR_OWNER_LAB_AVATAR_PROVIDER") {
        return Some(explicit.to_ascii_lowercase());
    }

    let did_complete = get("VPR_DID_API_KEY").is_some() && get("VPR_DID_AGENT_ID").is_some();
    let local_complete = get("VPR_LOCAL_AVATAR_ENDPOINT").is_some()
        && get("VPR_LOCAL_AVATAR_API_TOKEN").is_some();
    match (did_complete, local_complete) {
        (true, false) => Some("did".into()),
        (false, true) => Some("local-open-source".into()),
        _ => None,
    }
}

pub(super) fn avatar_provider_config_complete_with(
    get: &mut impl FnMut(&'static str) -> Option<String>,
) -> bool {
    let did_complete = get("VPR_DID_API_KEY").is_some() && get("VPR_DID_AGENT_ID").is_some();
    let local_complete = get("VPR_LOCAL_AVATAR_ENDPOINT").is_some()
        && get("VPR_LOCAL_AVATAR_API_TOKEN").is_some();
    let selected = get("VPR_OWNER_LAB_AVATAR_PROVIDER")
        .map(|value| value.to_ascii_lowercase())
        .or_else(|| match (did_complete, local_complete) {
            (true, false) => Some("did".into()),
            (false, true) => Some("local-open-source".into()),
            _ => None,
        });
    match selected.as_deref() {
        Some("did" | "d-id" | "did-agent-streams") => did_complete,
        Some("local" | "local-open-source") => local_complete,
        _ => false,
    }
}
