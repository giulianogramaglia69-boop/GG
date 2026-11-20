// SPDX-License-Identifier: GPL-3.0-or-later
//! Virtual environments for GG: a project directory with its own
//! `lib/` folder for modules and a `gg.toml` manifest.
//!
//! `gg venv <nome>` creates the directory; `gg run --venv <dir>`
//! (or the `GG_VENV` environment variable) adds `<dir>/lib` to the
//! module search path. A venv holds the *user's* modules — there is
//! no standard library yet, so nothing is installed into it.

use std::path::{Path, PathBuf};

/// Create a venv with the given name in the current directory:
/// `<name>/` with `lib/` inside and a `gg.toml` manifest (`name`
/// and `version`). An existing directory (or an invalid name) is
/// an error.
pub fn create_venv(name: &str) -> Result<(), String> {
    if name.is_empty() || name.contains('/') || name.contains('\\') || name == "." || name == ".." {
        return Err(format!("invalid venv name '{name}'"));
    }
    create_venv_at(Path::new(name))
}

/// Create a venv at the given directory (used by `create_venv`,
/// and by the tests with an absolute path so they never have to
/// change the process working directory).
pub fn create_venv_at(dir: &Path) -> Result<(), String> {
    if dir.exists() {
        return Err(format!("venv '{}' already exists", dir.display()));
    }
    let name = dir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    std::fs::create_dir_all(dir.join("lib"))
        .map_err(|e| format!("cannot create venv '{}': {e}", dir.display()))?;
    let manifest = format!("name = \"{name}\"\nversion = \"0.1.0\"\n");
    std::fs::write(dir.join("gg.toml"), manifest)
        .map_err(|e| format!("cannot write '{}/gg.toml': {e}", dir.display()))?;
    Ok(())
}

/// The `lib/` directory of a venv: where its modules live.
pub fn venv_lib_dir(venv_dir: &Path) -> PathBuf {
    venv_dir.join("lib")
}

/// The venv from the `GG_VENV` environment variable, if set: its
/// `lib/` directory. Unset or empty means "no venv". A path that
/// does not exist is ignored silently (the search just finds
/// nothing there); `--venv` on the command line is stricter and
/// errors on a missing directory instead.
pub fn venv_dirs_from_env() -> Vec<PathBuf> {
    match std::env::var("GG_VENV") {
        Ok(value) if !value.trim().is_empty() => {
            vec![venv_lib_dir(Path::new(value.trim()))]
        }
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fresh scratch directory for a venv test (unique per test
    /// tag, removed when the test is done).
    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("gg-test-venv-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn venv_creates_dir_lib_and_manifest() {
        let dir = temp_dir("create");
        let venv = dir.join("myproj");
        let result = create_venv_at(&venv);
        assert!(result.is_ok(), "{result:?}");
        assert!(venv.is_dir());
        assert!(venv.join("lib").is_dir());
        let manifest = std::fs::read_to_string(venv.join("gg.toml")).unwrap();
        assert_eq!(manifest, "name = \"myproj\"\nversion = \"0.1.0\"\n");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn venv_existing_directory_is_an_error() {
        let dir = temp_dir("exists");
        let venv = dir.join("taken");
        std::fs::create_dir_all(&venv).unwrap();
        let err = create_venv_at(&venv).unwrap_err();
        assert!(err.contains("already exists"), "{err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn venv_invalid_names_are_errors() {
        assert!(create_venv("").is_err());
        assert!(create_venv("a/b").is_err());
        assert!(create_venv("a\\b").is_err());
        assert!(create_venv(".").is_err());
        assert!(create_venv("..").is_err());
    }

    #[test]
    fn venv_lib_dir_joins_lib() {
        assert_eq!(
            venv_lib_dir(Path::new("/home/x/myproj")),
            PathBuf::from("/home/x/myproj/lib")
        );
    }

    #[test]
    fn gg_venv_env_var_gives_the_lib_dir() {
        // `GG_VENV` is process-wide, but no other test reads it.
        let dir = temp_dir("env");
        let venv = dir.join("myproj");
        std::fs::create_dir_all(venv.join("lib")).unwrap();
        let old = std::env::var("GG_VENV").ok();
        unsafe {
            std::env::set_var("GG_VENV", venv.to_str().unwrap());
        }
        let dirs = venv_dirs_from_env();
        match old {
            Some(value) => unsafe {
                std::env::set_var("GG_VENV", value);
            },
            None => unsafe {
                std::env::remove_var("GG_VENV");
            },
        }
        assert_eq!(dirs, vec![venv.join("lib")]);
        // Unset means no venv.
        let old = std::env::var("GG_VENV").ok();
        unsafe {
            std::env::remove_var("GG_VENV");
        }
        let dirs = venv_dirs_from_env();
        match old {
            Some(value) => unsafe {
                std::env::set_var("GG_VENV", value);
            },
            None => unsafe {
                std::env::remove_var("GG_VENV");
            },
        }
        assert!(dirs.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
