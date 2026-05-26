use std::path::Path;

pub fn is_sensitive_path(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(is_sensitive_name)
}

pub fn is_sensitive_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower == ".env"
        || lower.starts_with(".env.")
        || lower.ends_with(".pem")
        || lower.ends_with(".key")
        || lower == "id_rsa"
        || lower == "id_ed25519"
        || lower.starts_with("credentials")
        || lower.starts_with("secrets")
}

#[cfg(test)]
mod tests {
    use super::is_sensitive_name;

    #[test]
    fn detects_sensitive_names() {
        assert!(is_sensitive_name(".env"));
        assert!(is_sensitive_name(".env.local"));
        assert!(is_sensitive_name("server.key"));
        assert!(is_sensitive_name("id_ed25519"));
        assert!(is_sensitive_name("credentials.json"));
        assert!(!is_sensitive_name("main.rs"));
    }
}
