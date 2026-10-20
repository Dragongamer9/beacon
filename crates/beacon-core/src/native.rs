//! Windows: starting programs that ask for administrator rights.
//!
//! Disc installers (and a few old game exes) are marked "requires
//! administrator". Started directly, Windows refuses them with error 740
//! (`ERROR_ELEVATION_REQUIRED`); they have to go through the UAC prompt. Beacon
//! then starts them with PowerShell's `Start-Process -Verb RunAs -Wait` from a
//! fixed script file, with the program, folder and arguments passed as separate
//! parameters (`-File` binds them literally, nothing is parsed as code). The
//! returned child is PowerShell, which waits for the program and exits with its
//! exit code (1223 when the UAC prompt is declined).

/// `ERROR_ELEVATION_REQUIRED`.
pub const ERROR_ELEVATION_REQUIRED: i32 = 740;

/// The helper script (written to `<data>/tmp/beacon-elevate.ps1`).
pub const ELEVATE_SCRIPT: &str = r#"param([string]$Exe, [string]$Dir, [string]$ArgLine)
$ErrorActionPreference = 'Stop'
$p = @{ FilePath = $Exe; Verb = 'RunAs'; Wait = $true; PassThru = $true }
if ($Dir) { $p.WorkingDirectory = $Dir }
if ($ArgLine) { $p.ArgumentList = $ArgLine }
try { $proc = Start-Process @p } catch { exit 1223 }
exit $proc.ExitCode
"#;

#[cfg(windows)]
pub fn spawn_elevated(data_dir: &std::path::Path, cmd: &crate::runner::Command) -> anyhow::Result<std::process::Child> {
    use anyhow::Context;
    use std::path::PathBuf;
    let dir = data_dir.join("tmp");
    std::fs::create_dir_all(&dir)?;
    let script = dir.join("beacon-elevate.ps1");
    std::fs::write(&script, ELEVATE_SCRIPT)?;
    let root = std::env::var_os("SystemRoot").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
    let powershell = root.join("System32").join("WindowsPowerShell").join("v1.0").join("powershell.exe");
    let mut c = std::process::Command::new(&powershell);
    c.args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-WindowStyle", "Hidden", "-File"])
        .arg(&script)
        .arg(&cmd.program)
        .arg(&cmd.cwd)
        .arg(crate::runner::windows_command_line(&cmd.args))
        .current_dir(&cmd.cwd);
    for (k, v) in &cmd.env {
        c.env(k, v);
    }
    c.spawn().with_context(|| format!("starting {} with administrator rights", cmd.program.display()))
}

#[cfg(test)]
mod tests {
    #[test]
    fn script_takes_parameters_only() {
        let s = super::ELEVATE_SCRIPT;
        assert!(s.starts_with("param([string]$Exe, [string]$Dir, [string]$ArgLine)"));
        assert!(s.contains("-Verb") || s.contains("Verb = 'RunAs'"));
        assert!(!s.contains("Invoke-Expression") && !s.contains("iex "));
    }
}
