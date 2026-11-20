// SPDX-License-Identifier: GPL-3.0-or-later
//! CLI end-to-end tests: spawn the real `gg` binary in a scratch
//! directory and check `venv`, `run --venv`, `GG_VENV` and the
//! working-directory fallback. (Integration tests get
//! `CARGO_BIN_EXE_gg` from cargo; the binary is built before the
//! tests run.)

use std::path::PathBuf;
use std::process::Command as ProcCommand;

/// The built `gg` binary.
fn gg_bin() -> &'static str {
    env!("CARGO_BIN_EXE_gg")
}

/// A fresh scratch directory for a CLI test (unique per test tag,
/// removed when the test is done).
fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("gg-test-cli-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn cli_venv_creates_the_project() {
    let dir = temp_dir("venv");
    let output = ProcCommand::new(gg_bin())
        .args(["venv", "myproj"])
        .current_dir(&dir)
        .output()
        .unwrap();
    let status = output.status;
    let stdout = String::from_utf8(output.stdout).unwrap();
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(status.success(), "stdout={stdout} stderr={stderr}");
    assert!(dir.join("myproj/lib").is_dir());
    assert_eq!(
        std::fs::read_to_string(dir.join("myproj/gg.toml")).unwrap(),
        "name = \"myproj\"\nversion = \"0.1.0\"\n"
    );
    assert!(stdout.contains("Created venv 'myproj'"), "{stdout}");
    // A second `gg venv` with the same name fails.
    let output = ProcCommand::new(gg_bin())
        .args(["venv", "myproj"])
        .current_dir(&dir)
        .output()
        .unwrap();
    let status = output.status;
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(!status.success());
    assert!(stderr.contains("already exists"), "{stderr}");
    // No name at all is a usage error.
    let output = ProcCommand::new(gg_bin())
        .args(["venv"])
        .current_dir(&dir)
        .output()
        .unwrap();
    assert!(!output.status.success());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn cli_run_with_venv_flag_resolves_a_module_from_lib() {
    let dir = temp_dir("run-venv");
    let lib = dir.join("myproj").join("lib");
    std::fs::create_dir_all(&lib).unwrap();
    std::fs::write(
        lib.join("maths.gg"),
        "fn raddoppia(x)\n    return x * 2\nend\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("main.gg"),
        "import maths\nprint maths.raddoppia(21)\n",
    )
    .unwrap();
    let output = ProcCommand::new(gg_bin())
        .args(["run", "--venv", "myproj", "main.gg"])
        .current_dir(&dir)
        .output()
        .unwrap();
    let status = output.status;
    let stdout = String::from_utf8(output.stdout).unwrap();
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(status.success(), "stdout={stdout} stderr={stderr}");
    assert_eq!(stdout, "42\n");
    // A missing --venv directory is a clear error.
    let output = ProcCommand::new(gg_bin())
        .args(["run", "--venv", "nope", "main.gg"])
        .current_dir(&dir)
        .output()
        .unwrap();
    let status = output.status;
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(!status.success());
    assert!(
        stderr.contains("venv directory 'nope' does not exist"),
        "{stderr}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn cli_run_with_gg_venv_env_var() {
    let dir = temp_dir("run-ggvenv");
    let lib = dir.join("myproj").join("lib");
    std::fs::create_dir_all(&lib).unwrap();
    std::fs::write(
        lib.join("maths.gg"),
        "fn raddoppia(x)\n    return x * 2\nend\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("main.gg"),
        "import maths\nprint maths.raddoppia(21)\n",
    )
    .unwrap();
    // No --venv flag: GG_VENV supplies the venv.
    let output = ProcCommand::new(gg_bin())
        .args(["run", "main.gg"])
        .env("GG_VENV", "myproj")
        .current_dir(&dir)
        .output()
        .unwrap();
    let status = output.status;
    let stdout = String::from_utf8(output.stdout).unwrap();
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(status.success(), "stdout={stdout} stderr={stderr}");
    assert_eq!(stdout, "42\n");
    // --venv wins over GG_VENV when both are given: point the
    // flag at an empty venv and the module is not found.
    std::fs::create_dir_all(dir.join("empty").join("lib")).unwrap();
    let output = ProcCommand::new(gg_bin())
        .args(["run", "--venv", "empty", "main.gg"])
        .env("GG_VENV", "myproj")
        .current_dir(&dir)
        .output()
        .unwrap();
    let status = output.status;
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(!status.success());
    assert!(stderr.contains("module 'maths' not found"), "{stderr}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn cli_run_imports_from_the_working_directory() {
    let dir = temp_dir("run-cwd");
    std::fs::write(
        dir.join("helper.gg"),
        "fn chi()\n    return \"da cwd\"\nend\n",
    )
    .unwrap();
    std::fs::write(dir.join("main.gg"), "import helper\nprint helper.chi()\n").unwrap();
    let output = ProcCommand::new(gg_bin())
        .args(["run", "main.gg"])
        .current_dir(&dir)
        .output()
        .unwrap();
    let status = output.status;
    let stdout = String::from_utf8(output.stdout).unwrap();
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(status.success(), "stdout={stdout} stderr={stderr}");
    assert_eq!(stdout, "da cwd\n");
    let _ = std::fs::remove_dir_all(&dir);
}
