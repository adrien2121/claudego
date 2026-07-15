use crate::harness::SessionRoot;
use std::path::PathBuf;

pub struct ClaudeRoot;

impl ClaudeRoot {
    fn resolve_from(home: Option<PathBuf>) -> Option<PathBuf> {
        home.map(|path| path.join(".claude/projects"))
    }
}

impl SessionRoot for ClaudeRoot {
    fn resolve(&self) -> Option<PathBuf> {
        Self::resolve_from(dirs::home_dir())
    }
}

#[cfg(test)]
mod tests {
    use super::ClaudeRoot;
    use std::path::PathBuf;

    #[test]
    fn defaults_to_claude_projects_under_home() {
        assert_eq!(
            ClaudeRoot::resolve_from(Some(PathBuf::from("/home/test"))),
            Some(PathBuf::from("/home/test/.claude/projects")),
        );
    }
}
