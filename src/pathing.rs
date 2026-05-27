use std::path::Path;

use anyhow::{Context, Result, bail};
use camino::{Utf8Path, Utf8PathBuf};

pub fn canonical_utf8(path: &Path) -> Result<Utf8PathBuf> {
    let path =
        dunce::canonicalize(path).with_context(|| format!("invalid root {}", path.display()))?;
    Utf8PathBuf::from_path_buf(path)
        .map_err(|path| anyhow::anyhow!("non-utf8 path {}", path.display()))
}

pub fn safe_join(root: &Utf8Path, rel: &Utf8Path) -> Result<Utf8PathBuf> {
    if rel.is_absolute() || is_unsafe_relative_path(rel.as_str()) {
        bail!("unsafe path: {rel}");
    }
    Ok(root.join(rel))
}

fn is_unsafe_relative_path(path: &str) -> bool {
    path.starts_with('\\')
        || path.as_bytes().get(1).is_some_and(|byte| *byte == b':')
        || path
            .split(['/', '\\'])
            .any(|part| part == ".." || part.is_empty())
}

#[cfg(test)]
mod tests {
    use camino::Utf8Path;

    use super::safe_join;

    #[test]
    fn safe_join_blocks_parent_traversal() {
        let root = Utf8Path::new("C:/repo");
        assert!(safe_join(root, Utf8Path::new("../secret")).is_err());
    }

    #[test]
    fn safe_join_blocks_windows_absolute_forms() {
        let root = Utf8Path::new("C:/repo");
        assert!(safe_join(root, Utf8Path::new("C:/secret")).is_err());
        assert!(safe_join(root, Utf8Path::new("C:\\secret")).is_err());
        assert!(safe_join(root, Utf8Path::new("\\secret")).is_err());
    }

    #[test]
    fn safe_join_blocks_backslash_traversal() {
        let root = Utf8Path::new("C:/repo");
        assert!(safe_join(root, Utf8Path::new("..\\secret")).is_err());
        assert!(safe_join(root, Utf8Path::new("src\\..\\secret")).is_err());
    }

    #[test]
    fn safe_join_allows_normal_relative_paths() {
        let root = Utf8Path::new("C:/repo");
        assert!(safe_join(root, Utf8Path::new("src/main.rs")).is_ok());
        assert!(safe_join(root, Utf8Path::new("src\\main.rs")).is_ok());
    }
}
