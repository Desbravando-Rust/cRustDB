use std::fs::DirBuilder;
use std::os::unix::fs::DirBuilderExt;
use std::path::PathBuf;

// `Mac` is only built on macOS (and in tests): the table stays testable on Linux.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
#[derive(Clone, Copy)]
enum Os {
    Linux,
    Mac,
}

#[cfg(target_os = "macos")]
const CURRENT: Os = Os::Mac;
#[cfg(not(target_os = "macos"))]
const CURRENT: Os = Os::Linux;

#[derive(Clone, Copy)]
enum Dir {
    Config,
    Data,
    State,
}

/// (XDG variable, fallback under `$HOME`). XDG wins on every OS.
fn spec(os: Os, dir: Dir) -> (&'static str, &'static str) {
    match (dir, os) {
        (Dir::Config, Os::Linux) => ("XDG_CONFIG_HOME", ".config"),
        (Dir::Data, Os::Linux) => ("XDG_DATA_HOME", ".local/share"),
        (Dir::State, Os::Linux) => ("XDG_STATE_HOME", ".local/state"),
        (Dir::Config, Os::Mac) => ("XDG_CONFIG_HOME", "Library/Application Support"),
        (Dir::Data, Os::Mac) => ("XDG_DATA_HOME", "Library/Application Support"),
        (Dir::State, Os::Mac) => ("XDG_STATE_HOME", "Library/Logs"),
    }
}

fn resolve(env: impl Fn(&str) -> Option<String>, os: Os, dir: Dir) -> PathBuf {
    let (xdg, fallback) = spec(os, dir);
    let base = match env(xdg).filter(|v| !v.is_empty()) {
        Some(v) => PathBuf::from(v),
        None => PathBuf::from(env("HOME").unwrap_or_default()).join(fallback),
    };
    base.join("crustdb")
}

fn ensure(dir: PathBuf) -> PathBuf {
    // Best effort: callers surface the real error when they write into it.
    let _ = DirBuilder::new().recursive(true).mode(0o700).create(&dir);
    dir
}

fn sys(name: &str) -> Option<String> {
    std::env::var(name).ok()
}

pub fn config_dir() -> PathBuf {
    ensure(resolve(sys, CURRENT, Dir::Config))
}

pub fn data_dir() -> PathBuf {
    ensure(resolve(sys, CURRENT, Dir::Data))
}

pub fn state_dir() -> PathBuf {
    ensure(resolve(sys, CURRENT, Dir::State))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn xdg_wins_then_home_fallback() {
        // Arrange
        let xdg = |k: &str| (k == "XDG_DATA_HOME").then(|| "/x".to_string());

        // Act & Assert
        assert_eq!(
            resolve(xdg, Os::Linux, Dir::Data),
            PathBuf::from("/x/crustdb")
        );
        let home = |k: &str| (k == "HOME").then(|| "/h".to_string());
        assert_eq!(
            resolve(home, Os::Linux, Dir::State),
            PathBuf::from("/h/.local/state/crustdb")
        );
    }

    #[test]
    fn mac_falls_back_to_application_support() {
        // Arrange
        let home = |k: &str| (k == "HOME").then(|| "/h".to_string());

        // Act & Assert
        assert_eq!(
            resolve(home, Os::Mac, Dir::Config),
            PathBuf::from("/h/Library/Application Support/crustdb")
        );
        assert_eq!(
            resolve(home, Os::Mac, Dir::State),
            PathBuf::from("/h/Library/Logs/crustdb")
        );
    }

    #[test]
    fn xdg_still_wins_on_mac() {
        // Arrange
        let xdg = |k: &str| (k == "XDG_CONFIG_HOME").then(|| "/x".to_string());

        // Act & Assert
        assert_eq!(
            resolve(xdg, Os::Mac, Dir::Config),
            PathBuf::from("/x/crustdb")
        );
    }

    #[test]
    fn ensure_creates_0700() {
        // Arrange
        use std::os::unix::fs::PermissionsExt;
        let t = tempfile::tempdir().unwrap();

        // Act
        let d = ensure(t.path().join("a/crustdb"));

        // Assert
        assert_eq!(d.metadata().unwrap().permissions().mode() & 0o777, 0o700);
    }
}
