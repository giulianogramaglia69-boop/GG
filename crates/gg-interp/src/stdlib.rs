// SPDX-License-Identifier: GPL-3.0-or-later
//! Standard library modules written in GG, embedded in the binary.
//!
//! The files in `stdlib/` at the repo root are the source of truth;
//! they are compiled into the binary with `include_str!`, so `import
//! stats` works with no file on disk. A user file with the same name
//! still wins: the import search tries `.gg` files first (see
//! `Interpreter::import_module`).

/// The source of an embedded stdlib module, or `None` when GG has
/// no such module.
pub fn get(name: &str) -> Option<&'static str> {
    match name {
        "stats" => Some(include_str!("../../../stdlib/stats.gg")),
        _ => None,
    }
}

/// The names of all embedded stdlib modules.
pub fn names() -> &'static [&'static str] {
    &["stats"]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The embedded source must match the file on disk: the file
    /// is the source of truth, the binary just carries a copy.
    #[test]
    fn embedded_source_matches_the_files_on_disk() {
        let dir = format!("{}/../../stdlib", env!("CARGO_MANIFEST_DIR"));
        for name in names() {
            let path = format!("{dir}/{name}.gg");
            let on_disk = std::fs::read_to_string(&path)
                .unwrap_or_else(|e| panic!("cannot read {path}: {e}"));
            assert_eq!(get(name).unwrap(), on_disk, "stdlib/{name}.gg out of sync");
        }
        assert_eq!(get("nope"), None);
    }
}
