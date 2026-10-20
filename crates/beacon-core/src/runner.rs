//! Finding Wine, Proton and umu-launcher on the system, and turning
//! "run this exe in that prefix" into a concrete command.

use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RunnerKind {
    /// `umu-run` with its own auto-downloaded UMU-Proton.
    Umu,
    /// A Proton build directory (GE-Proton, Valve Proton…).
    Proton,
    /// A plain `wine` binary.
    Wine,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Runner {
    pub id: String,
    pub name: String,
    pub kind: RunnerKind,
    /// umu-run binary, Proton directory, or wine binary.
    pub path: PathBuf,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RunnerSet {
    pub runners: Vec<Runner>,
    /// When present, Proton builds are started through umu-run so they get
    /// the Steam Linux Runtime without needing Steam itself.
    pub umu: Option<PathBuf>,
}

impl RunnerSet {
    pub fn get(&self, id: &str) -> Option<&Runner> {
        self.runners.iter().find(|r| r.id == id)
    }

    /// GE-Proton first (it carries the most game fixes), then any Proton,
    /// then umu's own Proton, then system Wine.
    pub fn best(&self) -> Option<&Runner> {
        let ge = self.runners.iter().find(|r| r.kind == RunnerKind::Proton && r.name.to_ascii_lowercase().contains("ge-proton"));
        ge.or_else(|| self.runners.iter().find(|r| r.kind == RunnerKind::Proton))
            .or_else(|| self.runners.iter().find(|r| r.kind == RunnerKind::Umu))
            .or_else(|| self.runners.first())
    }
}

/// A process to start: program, arguments, extra environment and working directory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Command {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
    pub cwd: PathBuf,
}

impl Command {
    /// Shell-ish one-liner, for logs and dry runs.
    pub fn display(&self) -> String {
        let mut s = String::new();
        for (k, v) in &self.env {
            s.push_str(&format!("{k}={} ", quote(v)));
        }
        s.push_str(&quote(&self.program.to_string_lossy()));
        for a in &self.args {
            s.push(' ');
            s.push_str(&quote(a));
        }
        s
    }

    pub fn to_std(&self) -> std::process::Command {
        let mut c = std::process::Command::new(&self.program);
        c.args(&self.args).current_dir(&self.cwd);
        for (k, v) in &self.env {
            c.env(k, v);
        }
        c
    }
}

fn quote(s: &str) -> String {
    if !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || "-_./=:,+@".contains(c)) {
        s.to_string()
    } else {
        format!("'{}'", s.replace('\'', "'\\''"))
    }
}

/// Look up a binary on a PATH-style string.
pub fn which_in(bin: &str, path_env: &str) -> Option<PathBuf> {
    std::env::split_paths(path_env).map(|d| d.join(bin)).find(|p| p.is_file())
}

/// Compare "GE-Proton10-9" vs "GE-Proton10-10" the way a human would.
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    let (mut a, mut b) = (a.as_bytes(), b.as_bytes());
    loop {
        match (a.first(), b.first()) {
            (None, None) => return Ordering::Equal,
            (None, _) => return Ordering::Less,
            (_, None) => return Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let la = a.iter().take_while(|c| c.is_ascii_digit()).count();
                let lb = b.iter().take_while(|c| c.is_ascii_digit()).count();
                let na: u128 = std::str::from_utf8(&a[..la]).unwrap().parse().unwrap_or(0);
                let nb: u128 = std::str::from_utf8(&b[..lb]).unwrap().parse().unwrap_or(0);
                match na.cmp(&nb) {
                    Ordering::Equal => {
                        a = &a[la..];
                        b = &b[lb..];
                    }
                    o => return o,
                }
            }
            (Some(x), Some(y)) => match x.to_ascii_lowercase().cmp(&y.to_ascii_lowercase()) {
                Ordering::Equal => {
                    a = &a[1..];
                    b = &b[1..];
                }
                o => return o,
            },
        }
    }
}

fn proton_dirs(home: &Path) -> Vec<PathBuf> {
    let mut dirs = vec![
        home.join(".steam/root/compatibilitytools.d"),
        home.join(".local/share/Steam/compatibilitytools.d"),
        home.join(".var/app/com.valvesoftware.Steam/data/Steam/compatibilitytools.d"),
        home.join(".local/share/umu/compatibilitytools"),
        home.join(".local/share/Steam/steamapps/common"),
        home.join(".steam/root/steamapps/common"),
        PathBuf::from("/usr/share/steam/compatibilitytools.d"),
    ];
    dirs.dedup();
    dirs
}

/// Find every runner we can use.
pub fn discover(home: &Path, path_env: &str) -> RunnerSet {
    let mut set = RunnerSet::default();

    // Proton builds: any directory with an executable `proton` script inside.
    let mut protons: Vec<Runner> = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for base in proton_dirs(home) {
        let Ok(rd) = std::fs::read_dir(&base) else { continue };
        for e in rd.flatten() {
            let dir = e.path();
            if !dir.join("proton").is_file() {
                continue;
            }
            let canon = std::fs::canonicalize(&dir).unwrap_or(dir.clone());
            if !seen.insert(canon) {
                continue;
            }
            let name = e.file_name().to_string_lossy().to_string();
            protons.push(Runner { id: format!("proton:{name}"), name, kind: RunnerKind::Proton, path: dir });
        }
    }
    // GE builds first (most game fixes), each group newest first.
    let is_ge = |r: &Runner| r.name.to_ascii_lowercase().contains("ge-proton");
    protons.sort_by(|a, b| is_ge(b).cmp(&is_ge(a)).then_with(|| natural_cmp(&b.name, &a.name)));
    set.runners.extend(protons);

    if let Some(umu) = which_in("umu-run", path_env) {
        set.runners.push(Runner { id: "umu".into(), name: "umu-launcher (UMU-Proton)".into(), kind: RunnerKind::Umu, path: umu.clone() });
        set.umu = Some(umu);
    }

    if let Some(wine) = which_in("wine", path_env) {
        set.runners.push(Runner { id: "wine:system".into(), name: "System Wine".into(), kind: RunnerKind::Wine, path: wine });
    }
    // Lutris-managed Wine builds.
    if let Ok(rd) = std::fs::read_dir(home.join(".local/share/lutris/runners/wine")) {
        let mut lutris: Vec<Runner> = rd
            .flatten()
            .filter_map(|e| {
                let bin = e.path().join("bin/wine");
                bin.is_file().then(|| {
                    let name = e.file_name().to_string_lossy().to_string();
                    Runner { id: format!("wine:{name}"), name: format!("{name} (Lutris)"), kind: RunnerKind::Wine, path: bin }
                })
            })
            .collect();
        lutris.sort_by(|a, b| natural_cmp(&b.name, &a.name));
        set.runners.extend(lutris);
    }
    set
}

/// Build the command that runs `target` (a Linux exe path or a Wine builtin
/// like `regedit`) inside the prefix rooted at `prefix_root`.
///
/// Prefix layout is the same for every runner: the Wine prefix lives at
/// `<prefix_root>/pfx`, which is also where Proton puts it when
/// `STEAM_COMPAT_DATA_PATH=<prefix_root>`.
pub fn command_for(
    runner: &Runner,
    umu: Option<&Path>,
    home: &Path,
    prefix_root: &Path,
    target: &str,
    args: &[String],
    cwd: &Path,
) -> Command {
    let pfx = prefix_root.join("pfx").to_string_lossy().to_string();
    let mut env: Vec<(String, String)> = Vec::new();
    let mut full_args: Vec<String> = Vec::new();
    let program;

    match runner.kind {
        RunnerKind::Wine => {
            program = runner.path.clone();
            env.push(("WINEPREFIX".into(), pfx));
            env.push(("WINEDEBUG".into(), "-all".into()));
            // Skip the Mono/Gecko install prompts when the prefix is created.
            env.push(("WINEDLLOVERRIDES".into(), "mscoree,mshtml=".into()));
            full_args.push(target.into());
        }
        RunnerKind::Umu => {
            program = runner.path.clone();
            env.push(("WINEPREFIX".into(), pfx));
            env.push(("GAMEID".into(), "umu-0".into()));
            env.push(("STORE".into(), "none".into()));
            full_args.push(target.into());
        }
        RunnerKind::Proton => match umu {
            Some(umu) => {
                program = umu.to_path_buf();
                env.push(("WINEPREFIX".into(), pfx));
                env.push(("GAMEID".into(), "umu-0".into()));
                env.push(("STORE".into(), "none".into()));
                env.push(("PROTONPATH".into(), runner.path.to_string_lossy().into()));
                full_args.push(target.into());
            }
            None => {
                program = runner.path.join("proton");
                let steam = [home.join(".steam/steam"), home.join(".local/share/Steam")]
                    .into_iter()
                    .find(|p| p.is_dir())
                    .unwrap_or_else(|| prefix_root.to_path_buf());
                env.push(("STEAM_COMPAT_DATA_PATH".into(), prefix_root.to_string_lossy().into()));
                env.push(("STEAM_COMPAT_CLIENT_INSTALL_PATH".into(), steam.to_string_lossy().into()));
                full_args.push("run".into());
                full_args.push(target.into());
            }
        },
    }
    full_args.extend(args.iter().cloned());
    Command { program, args: full_args, env, cwd: cwd.to_path_buf() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn natural_sort() {
        assert_eq!(natural_cmp("GE-Proton10-10", "GE-Proton10-9"), Ordering::Greater);
        assert_eq!(natural_cmp("GE-Proton9-27", "GE-Proton10-1"), Ordering::Less);
        assert_eq!(natural_cmp("abc", "ABC"), Ordering::Equal);
    }

    fn touch_exec(p: &Path) {
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, b"#!/bin/sh\n").unwrap();
    }

    #[test]
    fn discovers_and_ranks() {
        let t = tempfile::tempdir().unwrap();
        let home = t.path().join("home");
        let bin = t.path().join("bin");
        touch_exec(&home.join(".steam/root/compatibilitytools.d/GE-Proton10-9/proton"));
        touch_exec(&home.join(".steam/root/compatibilitytools.d/GE-Proton10-12/proton"));
        touch_exec(&home.join(".local/share/Steam/steamapps/common/Proton 9.0 (Beta)/proton"));
        touch_exec(&bin.join("wine"));
        touch_exec(&bin.join("umu-run"));
        let set = discover(&home, &bin.to_string_lossy());
        let ids: Vec<_> = set.runners.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids[0], "proton:GE-Proton10-12");
        assert!(ids.contains(&"umu") && ids.contains(&"wine:system"));
        assert_eq!(set.best().unwrap().id, "proton:GE-Proton10-12");
        assert!(set.umu.is_some());
    }

    #[test]
    fn proton_without_umu_uses_compat_data() {
        let r = Runner { id: "p".into(), name: "GE".into(), kind: RunnerKind::Proton, path: "/opt/ge".into() };
        let c = command_for(&r, None, Path::new("/nohome"), Path::new("/data/pfx/bfme2"), "/g/lotrbfme2.exe", &["-win".into()], Path::new("/g"));
        assert_eq!(c.program, PathBuf::from("/opt/ge/proton"));
        assert_eq!(c.args, vec!["run", "/g/lotrbfme2.exe", "-win"]);
        assert!(c.env.contains(&("STEAM_COMPAT_DATA_PATH".into(), "/data/pfx/bfme2".into())));
    }

    #[test]
    fn proton_with_umu_sets_protonpath() {
        let r = Runner { id: "p".into(), name: "GE".into(), kind: RunnerKind::Proton, path: "/opt/ge".into() };
        let c = command_for(&r, Some(Path::new("/usr/bin/umu-run")), Path::new("/h"), Path::new("/d/rotwk"), "regedit", &[], Path::new("/d"));
        assert_eq!(c.program, PathBuf::from("/usr/bin/umu-run"));
        assert_eq!(c.args, vec!["regedit"]);
        assert!(c.env.contains(&("WINEPREFIX".into(), "/d/rotwk/pfx".into())));
        assert!(c.env.contains(&("PROTONPATH".into(), "/opt/ge".into())));
    }
}
