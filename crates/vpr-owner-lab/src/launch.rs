#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct LaunchOptions {
    pub(crate) allow_egress: bool,
    pub(crate) show_help: bool,
}

pub(crate) fn parse_launch_options<I, S>(args: I) -> Result<LaunchOptions, String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut options = LaunchOptions::default();
    for arg in args {
        match arg.as_ref() {
            "--allow-egress" => options.allow_egress = true,
            "--help" | "-h" => options.show_help = true,
            other => return Err(format!("unsupported argument `{other}`")),
        }
    }
    Ok(options)
}

pub(crate) fn resolve_egress_enabled(options: &LaunchOptions, env_value: Option<&str>) -> bool {
    options.allow_egress || env_value == Some("true")
}

pub(crate) fn print_usage() {
    println!("Usage: vpr-owner-lab [--allow-egress]");
    println!("  --allow-egress  Enable external provider calls for this process only.");
    println!(
        "                  Without this flag (or VPR_OWNER_LAB_ALLOW_EGRESS=true), egress stays disabled."
    );
}

#[cfg(test)]
mod tests {
    use super::{LaunchOptions, parse_launch_options, resolve_egress_enabled};

    #[test]
    fn owner_lab_stays_fail_closed_by_default() {
        let options = parse_launch_options(std::iter::empty::<&str>()).unwrap();
        assert_eq!(options, LaunchOptions::default());
        assert!(!resolve_egress_enabled(&options, None));
        assert!(!resolve_egress_enabled(&options, Some("false")));
    }

    #[test]
    fn explicit_cli_flag_enables_egress_for_this_launch() {
        let options = parse_launch_options(["--allow-egress"]).unwrap();
        assert!(options.allow_egress);
        assert!(!options.show_help);
        assert!(resolve_egress_enabled(&options, None));
    }

    #[test]
    fn legacy_environment_opt_in_remains_supported() {
        let options = parse_launch_options(std::iter::empty::<&str>()).unwrap();
        assert!(resolve_egress_enabled(&options, Some("true")));
    }

    #[test]
    fn help_is_explicit_and_does_not_enable_egress() {
        let options = parse_launch_options(["--help"]).unwrap();
        assert!(options.show_help);
        assert!(!options.allow_egress);
        assert!(!resolve_egress_enabled(&options, None));
    }

    #[test]
    fn unknown_launch_argument_is_rejected() {
        let error = parse_launch_options(["--persist-egress"]).unwrap_err();
        assert_eq!(error, "unsupported argument `--persist-egress`");
    }
}
