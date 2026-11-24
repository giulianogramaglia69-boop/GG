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

#[test]
fn cli_install_list_remove_and_reinstall_from_lockfile() {
    let dir = temp_dir("pkg-cli");
    // A package source next to the project, and a demo that
    // imports it. HOME is pointed at the scratch directory so
    // the global store (~/.gg/packages) lands there too.
    let src = dir.join("ciao-src");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(
        src.join("gg.toml"),
        "name = \"ciao\"\nversion = \"1.0.0\"\n",
    )
    .unwrap();
    std::fs::write(
        src.join("lib.gg"),
        "fn saluta(nome)\n    return \"Ciao \" + nome + \"!\"\nend\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("main.gg"),
        "import ciao\nprint ciao.saluta(\"Giuliano\")\n",
    )
    .unwrap();
    let run = |args: &[&str]| {
        ProcCommand::new(gg_bin())
            .args(args)
            .current_dir(&dir)
            .env("HOME", dir.to_str().unwrap())
            .output()
            .unwrap()
    };
    // Before the install the import fails.
    let output = run(&["run", "main.gg"]);
    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("module 'ciao' not found"), "{stderr}");
    // Install from the local path.
    let output = run(&["install", "./ciao-src"]);
    let stdout = String::from_utf8(output.stdout).unwrap();
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(output.status.success(), "stdout={stdout} stderr={stderr}");
    assert!(stdout.contains("Installed ciao 1.0.0"), "{stdout}");
    assert!(
        dir.join(".gg")
            .join("packages")
            .join("ciao")
            .join("1.0.0")
            .join("lib.gg")
            .is_file()
    );
    assert!(dir.join("gg.lock").is_file());
    // The import now works.
    let output = run(&["run", "main.gg"]);
    let stdout = String::from_utf8(output.stdout).unwrap();
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(output.status.success(), "stdout={stdout} stderr={stderr}");
    assert_eq!(stdout, "Ciao Giuliano!\n");
    // `gg list` shows it.
    let output = run(&["list"]);
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("ciao 1.0.0 (global)"), "{stdout}");
    // Wipe the store: `gg install` with no arguments brings it
    // back from the lockfile.
    std::fs::remove_dir_all(dir.join(".gg")).unwrap();
    let output = run(&["install"]);
    let stdout = String::from_utf8(output.stdout).unwrap();
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(output.status.success(), "stdout={stdout} stderr={stderr}");
    assert!(stdout.contains("Installed ciao 1.0.0"), "{stdout}");
    let output = run(&["run", "main.gg"]);
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "Ciao Giuliano!\n"
    );
    // `gg remove` takes it away again (and prunes the lockfile).
    let output = run(&["remove", "ciao"]);
    let stdout = String::from_utf8(output.stdout).unwrap();
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(output.status.success(), "stdout={stdout} stderr={stderr}");
    assert!(!dir.join(".gg").join("packages").join("ciao").exists());
    let output = run(&["run", "main.gg"]);
    assert!(!output.status.success());
    let output = run(&["remove", "ciao"]);
    assert!(!output.status.success());
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("not installed")
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn cli_install_rejects_a_directory_without_a_manifest() {
    let dir = temp_dir("pkg-cli-bad");
    let src = dir.join("nopkg");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(src.join("lib.gg"), "fn v()\n    return 1\nend\n").unwrap();
    let output = ProcCommand::new(gg_bin())
        .args(["install", "./nopkg"])
        .current_dir(&dir)
        .env("HOME", dir.to_str().unwrap())
        .output()
        .unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("no gg.toml found"), "{stderr}");
    let _ = std::fs::remove_dir_all(&dir);
}
