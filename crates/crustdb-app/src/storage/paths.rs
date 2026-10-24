use std::fs::DirBuilder;
use std::os::unix::fs::DirBuilderExt;
use std::path::PathBuf;

fn resolve(env: impl Fn(&str) -> Option<String>, xdg: &str, fallback: &str) -> PathBuf {
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
    ensure(resolve(sys, "XDG_CONFIG_HOME", ".config"))
}

pub fn data_dir() -> PathBuf {
    ensure(resolve(sys, "XDG_DATA_HOME", ".local/share"))
}

pub fn state_dir() -> PathBuf {
    ensure(resolve(sys, "XDG_STATE_HOME", ".local/state"))
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
            resolve(xdg, "XDG_DATA_HOME", ".local/share"),
            PathBuf::from("/x/crustdb")
        );
        let home = |k: &str| (k == "HOME").then(|| "/h".to_string());
        assert_eq!(
            resolve(home, "XDG_STATE_HOME", ".local/state"),
            PathBuf::from("/h/.local/state/crustdb")
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
