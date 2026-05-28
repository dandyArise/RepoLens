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
    Ok(resolve_in_root(root, rel)?.0)
}

pub fn resolve_in_root(root: &Utf8Path, path: &Utf8Path) -> Result<(Utf8PathBuf, Utf8PathBuf)> {
    if is_absolute_path(path) {
        let target = canonical_utf8(path.as_std_path())?;
        if !target.starts_with(root) {
            bail!("unsafe path outside root: {path}");
        }
        let rel = target
            .strip_prefix(root)
            .with_context(|| format!("failed to relativize {target} against {root}"))?
            .to_path_buf();
        return Ok((target, rel));
    }

    if is_unsafe_relative_path(path.as_str()) {
        bail!("unsafe path: {path}");
    }

    let joined = root.join(path);
    if joined.exists() {
        let target = canonical_utf8(joined.as_std_path())?;
        if !target.starts_with(root) {
            bail!("unsafe path outside root: {path}");
        }
        let rel = target
            .strip_prefix(root)
            .with_context(|| format!("failed to relativize {target} against {root}"))?
            .to_path_buf();
        Ok((target, rel))
    } else {
        Ok((joined, path.to_path_buf()))
    }
}

fn is_absolute_path(path: &Utf8Path) -> bool {
    path.is_absolute() || is_windows_absolute_path(path.as_str())
}

fn is_windows_absolute_path(path: &str) -> bool {
    path.starts_with('\\') || path.as_bytes().get(1).is_some_and(|byte| *byte == b':')
}

fn is_unsafe_relative_path(path: &str) -> bool {
    path.split(['/', '\\'])
        .any(|part| part == ".." || part.is_empty())
}

#[cfg(test)]
mod tests {
    use camino::Utf8Path;

    use super::{resolve_in_root, safe_join};

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

    #[test]
    fn resolve_in_root_allows_absolute_path_inside_root() {
        let dir = tempfile::tempdir().unwrap();
        let root = dunce::canonicalize(dir.path()).unwrap();
        std::fs::create_dir(root.join("src")).unwrap();
        let file = root.join("src").join("main.rs");
        std::fs::write(&file, "fn main() {}\n").unwrap();

        let root = camino::Utf8PathBuf::from_path_buf(root).unwrap();
        let file = camino::Utf8PathBuf::from_path_buf(file).unwrap();
        let (target, rel) = resolve_in_root(&root, &file).unwrap();

        assert_eq!(target, file);
        assert_eq!(rel, Utf8Path::new("src/main.rs"));
    }

    #[test]
    fn resolve_in_root_blocks_absolute_path_outside_root() {
        let root_dir = tempfile::tempdir().unwrap();
        let outside_dir = tempfile::tempdir().unwrap();
        let outside = outside_dir.path().join("secret.txt");
        std::fs::write(&outside, "secret").unwrap();

        let root =
            camino::Utf8PathBuf::from_path_buf(dunce::canonicalize(root_dir.path()).unwrap())
                .unwrap();
        let outside =
            camino::Utf8PathBuf::from_path_buf(dunce::canonicalize(outside).unwrap()).unwrap();

        assert!(resolve_in_root(&root, &outside).is_err());
    }
}
