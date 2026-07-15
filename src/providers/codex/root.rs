use crate::harness::SessionRoot;
use std::ffi::OsString;
use std::path::PathBuf;

pub struct CodexRoot;

impl CodexRoot {
    fn resolve_from(codex_home: Option<OsString>, home: Option<PathBuf>) -> Option<PathBuf> {
        codex_home
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .or_else(|| home.map(|path| path.join(".codex")))
            .map(|path| path.join("sessions"))
    }
}

impl SessionRoot for CodexRoot {
    fn resolve(&self) -> Option<PathBuf> {
        Self::resolve_from(std::env::var_os("CODEX_HOME"), dirs::home_dir())
    }
}

#[cfg(test)]
mod tests {
    use super::CodexRoot;
    use std::ffi::OsString;
    use std::path::PathBuf;

    #[test]
    fn codex_home_override_wins() {
        assert_eq!(
            CodexRoot::resolve_from(
                Some(OsString::from("/custom/codex")),
                Some(PathBuf::from("/home/test")),
            ),
            Some(PathBuf::from("/custom/codex/sessions")),
        );
    }

    #[test]
    fn default_uses_dot_codex_under_home() {
        assert_eq!(
            CodexRoot::resolve_from(None, Some(PathBuf::from("/home/test"))),
            Some(PathBuf::from("/home/test/.codex/sessions")),
        );
    }
}
