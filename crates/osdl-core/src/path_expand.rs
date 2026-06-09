//! Cross-platform path normalization for recipe YAMLs.
//!
//! Thin wrapper around the `shellexpand` crate. Expands `~`, `$VAR`, and
//! `${VAR}` references, then resolves the result against an explicit
//! *base directory* when the path is relative. The base directory is the
//! recipe YAML's own parent — that's the only stable reference point a
//! config file has across machines: CWD changes between an interactive
//! shell and a launchd-spawned `.app`, but `dirname(config.yaml)` does
//! not.
//!
//! Variable expansion semantics follow shellexpand's bash-style defaults:
//! unset variables expand to empty, `$$` is a literal `$`, `~user` is
//! supported but discouraged in recipe files. Errors from shellexpand
//! (e.g. `${UNCLOSED`) cause the input to pass through verbatim — the
//! next syscall will surface the typo more clearly than a load-time
//! failure would.

use std::path::{Path, PathBuf};

/// Expand env vars and `~` in `input`, then resolve relative results against
/// `base_dir`. Absolute paths (after expansion) are returned unchanged.
pub fn expand(input: &str, base_dir: &Path) -> PathBuf {
    let expanded = expand_vars(input);
    let p = Path::new(&expanded);
    if p.is_absolute() {
        return p.to_path_buf();
    }
    base_dir.join(p)
}

/// Expand env vars and `~` in `input` without resolving relatives.
/// Useful for fields where the caller wants the raw expanded string
/// (e.g. CLI args that go through `std::fs::canonicalize` later).
pub fn expand_vars(input: &str) -> String {
    // shellexpand::full handles both `~` and `$VAR` / `${VAR}` and reads
    // from std::env. On a malformed pattern we keep the original input
    // — losing user typos to a hard error here is more confusing than
    // letting the open syscall fail with a clearer "no such file".
    shellexpand::full(input)
        .map(|cow| cow.into_owned())
        .unwrap_or_else(|_| input.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn absolute_passes_through() {
        let r = expand("/tmp/foo", Path::new("/base"));
        assert_eq!(r, PathBuf::from("/tmp/foo"));
    }

    #[test]
    fn relative_joins_base() {
        let r = expand("registry/unilabos", Path::new("/etc/osdl/recipes"));
        assert_eq!(r, PathBuf::from("/etc/osdl/recipes/registry/unilabos"));
    }

    #[test]
    fn tilde_expands_to_home() {
        std::env::set_var("HOME", "/home/alice");
        let r = expand("~/lab/registry", Path::new("/anywhere"));
        assert_eq!(r, PathBuf::from("/home/alice/lab/registry"));
    }

    #[test]
    fn dollar_brace_var() {
        std::env::set_var("LAB_DIR", "/var/lab");
        let r = expand("${LAB_DIR}/registry", Path::new("/anywhere"));
        assert_eq!(r, PathBuf::from("/var/lab/registry"));
    }

    #[test]
    fn bare_dollar_var_with_separator() {
        std::env::set_var("FOO", "/srv");
        let r = expand("$FOO/data", Path::new("/anywhere"));
        assert_eq!(r, PathBuf::from("/srv/data"));
    }
}
