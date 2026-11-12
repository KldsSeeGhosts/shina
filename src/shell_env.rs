//! Login-shell PATH reconciliation for macOS and Linux.
//!
//! Ported from KSG Code's `apps/server/src/os-jank.ts`,
//! `apps/desktop/src/shell/DesktopShellEnvironment.ts`, and `packages/shared/src/shell.ts`.
//! Only `PATH` is reconciled; no other variables or credentials are imported.

#[cfg(unix)]
pub fn reconcile_path() {
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    {
        let inherited = std::env::var_os("PATH").and_then(|value| value.into_string().ok());
        let shell_path = read_login_shell_path();
        let launchctl_path = if cfg!(target_os = "macos") && shell_path.is_none() {
            read_launchctl_path()
        } else {
            None
        };
        if let Some(merged) = merge_path_entries(
            shell_path.as_deref().or(launchctl_path.as_deref()),
            inherited.as_deref(),
        ) {
            // SAFETY: Called once at process start in `main()` before spawning worker threads.
            unsafe {
                std::env::set_var("PATH", merged);
            }
        }
    }
}

#[cfg(not(unix))]
pub fn reconcile_path() {}

#[cfg(any(target_os = "macos", target_os = "linux", test))]
const START_MARKER: &str = "__SHINA_ENV_PATH_START__";
#[cfg(any(target_os = "macos", target_os = "linux", test))]
const END_MARKER: &str = "__SHINA_ENV_PATH_END__";

#[cfg(any(target_os = "macos", target_os = "linux", test))]
fn trim_non_empty(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

#[cfg(any(target_os = "macos", target_os = "linux", test))]
pub(crate) fn list_login_shell_candidates(
    env_shell: Option<&str>,
    passwd_shell: Option<&str>,
    is_macos: bool,
) -> Vec<String> {
    let fallback = if is_macos {
        Some("/bin/zsh")
    } else {
        Some("/bin/bash")
    };
    let mut seen = std::collections::HashSet::new();
    let mut candidates = Vec::new();
    for candidate in [
        trim_non_empty(env_shell),
        trim_non_empty(passwd_shell),
        fallback.map(str::to_owned),
    ]
    .into_iter()
    .flatten()
    {
        if seen.insert(candidate.clone()) {
            candidates.push(candidate);
        }
    }
    candidates
}

#[cfg(any(target_os = "macos", target_os = "linux", test))]
pub(crate) fn build_path_capture_command(shell: &str) -> String {
    let basename = std::path::Path::new(shell)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(shell)
        .to_ascii_lowercase();
    let capture = if basename.starts_with("nu") {
        let default_value = "if ($env.PATH? | describe | str starts-with 'list') { \
            $env.PATH | each {|p| if ($p | str starts-with '~') { \
            $p | path expand --no-symlink } else { $p } } | str join (char esep) \
            } else { $env.PATH? | default '' }";
        let value = format!(
            "if $env.PATH? == null {{ '' }} else if \
             $env.ENV_CONVERSIONS?.PATH?.to_string? != null {{ \
             $env.PATH | do $env.ENV_CONVERSIONS.PATH.to_string $env.PATH \
             }} else {{ {default_value} }}"
        );
        format!("print '{START_MARKER}'; print ({value}); print '{END_MARKER}'")
    } else {
        format!(
            "printf '%s\\n' '{START_MARKER}'; printenv PATH || true; printf '%s\\n' '{END_MARKER}'"
        )
    };
    if matches!(basename.as_str(), "bash" | "zsh" | "sh" | "ksh" | "ksh93") {
        format!("(trap - DEBUG) 2>/dev/null && trap - DEBUG; {capture}")
    } else {
        capture
    }
}

#[cfg(any(target_os = "macos", target_os = "linux", test))]
pub(crate) fn extract_path_from_output(output: &str) -> Option<String> {
    let start = output.find(START_MARKER)?;
    let value_start = start + START_MARKER.len();
    let end = output[value_start..].find(END_MARKER)? + value_start;
    let value = output[value_start..end]
        .strip_prefix("\r\n")
        .or_else(|| output[value_start..end].strip_prefix('\n'))
        .unwrap_or(&output[value_start..end]);
    let value = value
        .strip_suffix("\r\n")
        .or_else(|| value.strip_suffix('\n'))
        .unwrap_or(value)
        .trim();
    (!value.is_empty()).then(|| value.to_owned())
}

#[cfg(any(target_os = "macos", target_os = "linux", test))]
pub(crate) fn merge_path_entries(
    preferred: Option<&str>,
    inherited: Option<&str>,
) -> Option<String> {
    let mut merged = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for path_value in [preferred, inherited].into_iter().flatten() {
        for entry in path_value.split(':') {
            let trimmed = entry.trim();
            if trimmed.is_empty() || !seen.insert(trimmed.to_owned()) {
                continue;
            }
            merged.push(trimmed.to_owned());
        }
    }
    (!merged.is_empty()).then(|| merged.join(":"))
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn read_user_login_shell() -> Option<String> {
    let uid = unsafe { libc::geteuid() };
    let passwd = unsafe { libc::getpwuid(uid) };
    if passwd.is_null() {
        return None;
    }
    let shell_ptr = unsafe { (*passwd).pw_shell };
    if shell_ptr.is_null() {
        return None;
    }
    let shell = unsafe { std::ffi::CStr::from_ptr(shell_ptr) }
        .to_str()
        .ok()?;
    trim_non_empty(Some(shell))
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn read_login_shell_path() -> Option<String> {
    let env_shell = std::env::var("SHELL").ok();
    let passwd_shell = read_user_login_shell();
    let candidates = list_login_shell_candidates(
        env_shell.as_deref(),
        passwd_shell.as_deref(),
        cfg!(target_os = "macos"),
    );
    for shell in candidates {
        let capture = build_path_capture_command(&shell);
        let mut command = std::process::Command::new(&shell);
        command.args(["-ilc", &capture]);
        if let Ok(output) =
            crate::process::output(&mut command, std::time::Duration::from_secs(5), 128 * 1024)
            && output.status.success()
        {
            let stdout = String::from_utf8_lossy(&output.stdout);
            if let Some(path) = extract_path_from_output(&stdout) {
                return Some(path);
            }
        }
    }
    None
}

#[cfg(target_os = "macos")]
fn read_launchctl_path() -> Option<String> {
    let mut command = std::process::Command::new("/bin/launchctl");
    command.args(["getenv", "PATH"]);
    let output =
        crate::process::output(&mut command, std::time::Duration::from_secs(2), 64 * 1024).ok()?;
    if !output.status.success() {
        return None;
    }
    trim_non_empty(Some(&String::from_utf8_lossy(&output.stdout)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn login_shell_candidates_deduplicate_and_include_platform_fallback() {
        assert_eq!(
            list_login_shell_candidates(Some(" /bin/zsh "), Some("/bin/zsh"), true),
            vec!["/bin/zsh".to_string()]
        );
        assert_eq!(
            list_login_shell_candidates(Some("/opt/homebrew/bin/fish"), Some("/bin/zsh"), true),
            vec!["/opt/homebrew/bin/fish".to_string(), "/bin/zsh".to_string(),]
        );
        assert_eq!(
            list_login_shell_candidates(None, None, false),
            vec!["/bin/bash".to_string()]
        );
    }

    #[test]
    fn extract_path_from_output_parses_markers_and_ignores_banner_noise() {
        let noisy = "Welcome to zsh!\n__SHINA_ENV_PATH_START__\n/opt/homebrew/bin:/usr/bin:/bin\n__SHINA_ENV_PATH_END__\nbye\n";
        assert_eq!(
            extract_path_from_output(noisy).as_deref(),
            Some("/opt/homebrew/bin:/usr/bin:/bin")
        );
        assert_eq!(
            extract_path_from_output("__SHINA_ENV_PATH_START__\n\n__SHINA_ENV_PATH_END__"),
            None
        );
        assert_eq!(extract_path_from_output("no markers"), None);
    }

    #[test]
    fn merge_path_entries_preserves_order_and_deduplicates() {
        assert_eq!(
            merge_path_entries(
                Some("/opt/homebrew/bin: /usr/local/bin :/usr/bin"),
                Some("/usr/bin:/bin:/opt/homebrew/bin:/usr/sbin"),
            )
            .as_deref(),
            Some("/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin")
        );
        assert_eq!(
            merge_path_entries(None, Some("/usr/bin:/bin")).as_deref(),
            Some("/usr/bin:/bin")
        );
        assert_eq!(merge_path_entries(None, None), None);
    }
}
