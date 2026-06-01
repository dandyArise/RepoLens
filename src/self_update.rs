use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};

const POWERSHELL_INSTALLER_URL: &str =
    "https://raw.githubusercontent.com/dandyArise/RepoLens/main/install/install.ps1";
const SHELL_INSTALLER_URL: &str =
    "https://raw.githubusercontent.com/dandyArise/RepoLens/main/install/install.sh";

pub fn run(version: &str, install_dir: Option<PathBuf>) -> Result<()> {
    let install_dir = install_dir.unwrap_or(default_install_dir()?);
    if cfg!(windows) {
        run_windows(version, &install_dir)
    } else {
        run_unix(version, &install_dir)
    }
}

fn default_install_dir() -> Result<PathBuf> {
    let exe = std::env::current_exe().context("failed to locate current executable")?;
    exe.parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| anyhow::anyhow!("failed to locate install directory"))
}

fn run_windows(version: &str, install_dir: &Path) -> Result<()> {
    let pid = std::process::id();
    let version = powershell_quote(version);
    let install_dir = powershell_quote(&install_dir.display().to_string());
    let url = powershell_quote(&cache_busted_url(POWERSHELL_INSTALLER_URL));
    let command = format!(
        "$ErrorActionPreference = 'Stop'; \
         $process = Get-Process -Id {pid} -ErrorAction SilentlyContinue; \
         if ($process) {{ Wait-Process -Id {pid} -ErrorAction SilentlyContinue }}; \
         $tmp = Join-Path ([System.IO.Path]::GetTempPath()) ('repolens-self-update-' + [System.Guid]::NewGuid() + '.ps1'); \
         Invoke-WebRequest -UseBasicParsing -Uri {url} -OutFile $tmp; \
         try {{ & $tmp -Action update -Version {version} -InstallDir {install_dir} }} \
         finally {{ Remove-Item -LiteralPath $tmp -Force -ErrorAction SilentlyContinue }}"
    );

    Command::new("powershell")
        .args([
            "-NoProfile",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            &command,
        ])
        .spawn()
        .context("failed to start background self-update")?;

    println!(
        "Self-update started in background. It will replace repolens after this process exits."
    );
    println!("Run `repolens --help` again in a few seconds to verify the installed command list.");
    Ok(())
}

fn run_unix(version: &str, install_dir: &Path) -> Result<()> {
    let script = unix_update_script(std::process::id(), version, install_dir);
    Command::new("sh")
        .args(["-c", &script])
        .spawn()
        .context("failed to start background self-update")?;

    println!(
        "Self-update started in background. It will replace repolens after this process exits."
    );
    println!("Run `repolens --help` again in a few seconds to verify the installed command list.");
    Ok(())
}

fn unix_update_script(pid: u32, version: &str, install_dir: &Path) -> String {
    format!(
        "if kill -0 {pid} 2>/dev/null; then \
           while kill -0 {pid} 2>/dev/null; do sleep 0.1; done; \
         fi; \
         tmp=\"$(mktemp)\" && \
         curl -fsSL {url} -o \"$tmp\" && \
         REPOLENS_ACTION=update REPOLENS_VERSION={version} REPOLENS_INSTALL_DIR={install_dir} sh \"$tmp\"; \
         status=$?; rm -f \"$tmp\"; exit $status",
        url = shell_quote(&cache_busted_url(SHELL_INSTALLER_URL)),
        version = shell_quote(version),
        install_dir = shell_quote(&install_dir.display().to_string()),
    )
}

fn cache_busted_url(url: &str) -> String {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default();
    let sep = if url.contains('?') { '&' } else { '?' };
    format!("{url}{sep}cache_bust={stamp}")
}

fn powershell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{cache_busted_url, powershell_quote, shell_quote, unix_update_script};

    #[test]
    fn quotes_powershell_arguments() {
        assert_eq!(powershell_quote("C:\\a'b"), "'C:\\a''b'");
    }

    #[test]
    fn quotes_shell_arguments() {
        assert_eq!(shell_quote("a'b"), "'a'\"'\"'b'");
    }

    #[test]
    fn unix_update_script_waits_for_current_process() {
        let script = unix_update_script(123, "latest", Path::new("/tmp/repolens"));

        assert!(script.contains("kill -0 123"));
        assert!(script.contains("while kill -0 123"));
        assert!(script.contains("REPOLENS_ACTION=update"));
        assert!(script.contains("REPOLENS_INSTALL_DIR='/tmp/repolens'"));
        assert!(script.contains("cache_bust="));
    }

    #[test]
    fn adds_cache_buster_to_installer_url() {
        let url = cache_busted_url("https://example.com/install.ps1");

        assert!(url.starts_with("https://example.com/install.ps1?cache_bust="));
    }
}
