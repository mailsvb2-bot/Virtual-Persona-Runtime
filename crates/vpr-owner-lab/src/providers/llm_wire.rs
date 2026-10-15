//! Operator-verified protocol variants for Russian OpenAI-compatible LLMs.
//! All selected wire controls participate in the canonical descriptor fingerprint.
use vpr_provider_openai_compatible::{AuthScheme, OpenAiCompatibleConfig, ProjectHeader};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct LlmWire {
    pub auth: AuthScheme,
    pub project_header: ProjectHeader,
    pub stream_usage: bool,
}

pub(super) fn resolve_wire(
    canonical: &str,
    mut get: impl FnMut(&'static str) -> Option<String>,
) -> Result<LlmWire, String> {
    let mut wire = match canonical {
        "yandex" => LlmWire {
            auth: AuthScheme::ApiKey,
            project_header: ProjectHeader::OpenAiProject,
            stream_usage: false,
        },
        "proxyapi-deepseek" => LlmWire {
            auth: AuthScheme::Bearer,
            project_header: ProjectHeader::OpenAiProject,
            stream_usage: false,
        },
        _ => LlmWire {
            auth: AuthScheme::Bearer,
            project_header: ProjectHeader::OpenAiProject,
            stream_usage: true,
        },
    };
    for name in [
        "VPR_OWNER_LAB_LLM_AUTH_SCHEME",
        "VPR_OWNER_LAB_LLM_PROJECT_HEADER",
        "VPR_OWNER_LAB_LLM_STREAM_USAGE",
    ] {
        let Some(value) = get(name).map(|s| s.trim().to_ascii_lowercase()) else { continue };
        if !matches!(canonical, "yandex" | "proxyapi-deepseek")
            || (canonical != "yandex" && name != "VPR_OWNER_LAB_LLM_STREAM_USAGE")
        {
            return Err(format!("LLM_WIRE_OVERRIDE_UNSUPPORTED: {name}"));
        }
        match (name, value.as_str()) {
            ("VPR_OWNER_LAB_LLM_AUTH_SCHEME", "apikey") => wire.auth = AuthScheme::ApiKey,
            ("VPR_OWNER_LAB_LLM_AUTH_SCHEME", "bearer") => wire.auth = AuthScheme::Bearer,
            ("VPR_OWNER_LAB_LLM_PROJECT_HEADER", "openai-project") => {
                wire.project_header = ProjectHeader::OpenAiProject;
            }
            ("VPR_OWNER_LAB_LLM_PROJECT_HEADER", "x-folder-id") => {
                wire.project_header = ProjectHeader::XFolderId;
            }
            ("VPR_OWNER_LAB_LLM_STREAM_USAGE", "on") => wire.stream_usage = true,
            ("VPR_OWNER_LAB_LLM_STREAM_USAGE", "off") => wire.stream_usage = false,
            _ => return Err(format!("LLM_WIRE_OVERRIDE_INVALID: {name}")),
        }
    }
    Ok(wire)
}

pub(super) fn apply_wire(config: OpenAiCompatibleConfig, wire: LlmWire) -> OpenAiCompatibleConfig {
    config
        .with_auth_scheme(wire.auth)
        .with_project_header(wire.project_header)
        .with_stream_usage(wire.stream_usage)
}

pub(super) fn realtime_profile(canonical: &str, wire: LlmWire) -> String {
    let usage = if wire.stream_usage { "on" } else { "off" };
    let auth = match wire.auth {
        AuthScheme::ApiKey => "apikey",
        AuthScheme::Bearer => "bearer",
    };
    match canonical {
        "deepseek" => format!("thinking=disabled;reasoning=none;max_tokens=96;stream_usage={usage};auth={auth}"),
        "yandex" => {
            let header = match wire.project_header {
                ProjectHeader::OpenAiProject => "",
                ProjectHeader::XFolderId => ";project_header=x-folder-id",
            };
            format!("max_tokens=96;stream_usage={usage};auth={auth};project=set{header}")
        }
        "proxyapi-deepseek" | "openai-compatible" =>
            format!("max_tokens=96;stream_usage={usage};auth={auth}"),
        "anthropic" | "gemini" => "max_tokens=96".to_owned(),
        _ => "provider-default".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn none(_: &'static str) -> Option<String> { None }

    #[test]
    fn default_wires_preserve_existing_provider_fingerprints() {
        assert_eq!(realtime_profile("yandex", resolve_wire("yandex", none).unwrap()), "max_tokens=96;stream_usage=off;auth=apikey;project=set");
        assert_eq!(realtime_profile("proxyapi-deepseek", resolve_wire("proxyapi-deepseek", none).unwrap()), "max_tokens=96;stream_usage=off;auth=bearer");
    }

    #[test]
    fn override_is_bound_and_unknown_values_fail_closed() {
        let w = resolve_wire("yandex", |name| match name {
            "VPR_OWNER_LAB_LLM_AUTH_SCHEME" => Some("bearer".into()),
            "VPR_OWNER_LAB_LLM_PROJECT_HEADER" => Some("x-folder-id".into()),
            "VPR_OWNER_LAB_LLM_STREAM_USAGE" => Some("on".into()),
            _ => None,
        }).unwrap();
        assert_eq!(realtime_profile("yandex", w), "max_tokens=96;stream_usage=on;auth=bearer;project=set;project_header=x-folder-id");
        assert!(resolve_wire("deepseek", |_| Some("on".into())).is_err());
        assert!(resolve_wire("yandex", |name| (name == "VPR_OWNER_LAB_LLM_AUTH_SCHEME").then(|| "basic".into())).is_err());
        assert!(resolve_wire("proxyapi-deepseek", |name| (name == "VPR_OWNER_LAB_LLM_AUTH_SCHEME").then(|| "apikey".into())).is_err());
    }
}
