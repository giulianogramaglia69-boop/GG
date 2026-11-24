// SPDX-License-Identifier: GPL-3.0-or-later
//! Package manager v1 for GG: `gg.toml` manifests, `gg install` /
//! `gg list` / `gg remove`, a `gg.lock` lockfile, and import
//! resolution for installed packages.
//!
//! Layout: a package lives in `<root>/<name>/<version>/` with a
//! `gg.toml` manifest and `.gg` files (`entry`, default
//! `lib.gg`, is what `import <name>` loads). Roots are the
//! project-local `gg_packages/` (searched first) and the global
//! `~/.gg/packages/`. There is no central registry in v1:
//! packages come from a local directory or from GitHub
//! (`owner/repo[@tag|@commit]`, cloned with `git`).
//!
//! `gg install` only downloads and copies code — it never runs
//! anything (no install hooks, by design). Importing a package
//! runs its GG code exactly like any other `import`, so a
//! package is trusted the way any dependency is trusted.
//!
//! Limits (v1): no registry, no version resolver (two
//! explicitly demanded versions of the same package is an
//! error), no `gg publish`, no `gg update`, no signatures.
//! Versions are plain `X.Y.Z` (no prerelease or build
//! metadata).

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt;
use std::path::{Path, PathBuf};
use std::process::Command as ProcCommand;
use std::rc::Rc;

use crate::json;
use crate::value::{DictData, DictKey, Value};

/// A semantic version `X.Y.Z` (plain: no prerelease or build
/// metadata in v1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SemVer {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
}

impl SemVer {
    /// Parse `X.Y.Z` (all three numbers required).
    pub fn parse(text: &str) -> Result<SemVer, String> {
        let parts: Vec<&str> = text.split('.').collect();
        if parts.len() != 3 {
            return Err(format!("invalid version '{text}': expected X.Y.Z"));
        }
        let mut nums = [0u64; 3];
        for (i, part) in parts.iter().enumerate() {
            if part.is_empty() || !part.chars().all(|c| c.is_ascii_digit()) {
                return Err(format!("invalid version '{text}': expected X.Y.Z"));
            }
            nums[i] = part
                .parse()
                .map_err(|_| format!("invalid version '{text}': number too large"))?;
        }
        Ok(SemVer {
            major: nums[0],
            minor: nums[1],
            patch: nums[2],
        })
    }

    /// Parse a version tag: a leading `v` is stripped, so
    /// `v1.2.3` parses as 1.2.3. `None` when it is not a
    /// version at all.
    pub fn parse_tag(text: &str) -> Option<SemVer> {
        SemVer::parse(text.strip_prefix('v').unwrap_or(text)).ok()
    }
}

impl fmt::Display for SemVer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

/// True when `name` can be imported (`import <name>`): a GG
/// identifier (letter or `_` first, then letters, digits, `_`).
pub fn is_valid_package_name(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// A `gg.toml` manifest: the package's identity, its entry
/// point and its dependencies.
#[derive(Debug, Clone)]
pub struct Manifest {
    pub name: String,
    pub version: SemVer,
    pub description: Option<String>,
    /// The file `import <name>` loads (relative to the package
    /// root, default `lib.gg`).
    pub entry: String,
    pub dependencies: Vec<PackageSpec>,
}

/// Parse a `gg.toml` manifest. TOML errors carry line and
/// column (from the `toml` crate); missing or mistyped fields
/// are named plainly.
pub fn parse_manifest(text: &str) -> Result<Manifest, String> {
    let table: toml::Table = text.parse().map_err(|e| format!("gg.toml: {e}"))?;
    let name = required_str(&table, "name")?;
    if !is_valid_package_name(&name) {
        return Err(format!(
            "gg.toml: invalid package name '{name}' (must be a GG identifier, e.g. 'ciao')"
        ));
    }
    let version_text = required_str(&table, "version")?;
    let version = SemVer::parse(&version_text).map_err(|e| format!("gg.toml: {e}"))?;
    let description = optional_str(&table, "description")?;
    let entry = optional_str(&table, "entry")?.unwrap_or_else(|| "lib.gg".to_string());
    if entry.is_empty()
        || Path::new(&entry).is_absolute()
        || entry.split('/').any(|part| part == "..")
        || !entry.ends_with(".gg")
    {
        return Err(format!(
            "gg.toml: invalid entry '{entry}' (must be a relative .gg path inside the package)"
        ));
    }
    let dependencies = match table.get("dependencies") {
        None => Vec::new(),
        Some(toml::Value::Array(items)) => {
            let mut deps = Vec::new();
            for item in items {
                let Some(spec) = item.as_str() else {
                    return Err("gg.toml: 'dependencies' must be a list of strings".to_string());
                };
                deps.push(
                    PackageSpec::parse(spec)
                        .map_err(|e| format!("gg.toml: dependency '{spec}': {e}"))?,
                );
            }
            deps
        }
        Some(_) => {
            return Err("gg.toml: 'dependencies' must be a list of strings".to_string());
        }
    };
    Ok(Manifest {
        name,
        version,
        description,
        entry,
        dependencies,
    })
}

fn required_str(table: &toml::Table, key: &str) -> Result<String, String> {
    match table.get(key) {
        Some(toml::Value::String(value)) => Ok(value.clone()),
        Some(_) => Err(format!("gg.toml: '{key}' must be a string")),
        None => Err(format!("gg.toml: missing required field '{key}'")),
    }
}

fn optional_str(table: &toml::Table, key: &str) -> Result<Option<String>, String> {
    match table.get(key) {
        None => Ok(None),
        Some(toml::Value::String(value)) => Ok(Some(value.clone())),
        Some(_) => Err(format!("gg.toml: '{key}' must be a string")),
    }
}

/// The `@ref` of a GitHub spec: a tag, or a commit hash.
/// A ref of 7–40 hexadecimal characters is read as a commit;
/// anything else is a tag — so a tag that happens to be 7–40
/// hex characters is misread as a commit (use a longer,
/// non-hex tag name to avoid that).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GitRef {
    Tag(String),
    Commit(String),
}

/// Where a package comes from: a GitHub repository, or a
/// directory on disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PackageSpec {
    GitHub {
        owner: String,
        repo: String,
        git_ref: Option<GitRef>,
    },
    Local {
        path: PathBuf,
    },
}

impl PackageSpec {
    /// Parse a spec: `owner/repo`, `owner/repo@tag`,
    /// `owner/repo@<commit>` or a local path (`./dir`,
    /// `../dir`, `/absolute`).
    pub fn parse(text: &str) -> Result<PackageSpec, String> {
        let text = text.trim();
        if text.is_empty() {
            return Err("empty package spec".to_string());
        }
        if text.starts_with("./")
            || text.starts_with("../")
            || text.starts_with('/')
            || text == "."
            || text == ".."
        {
            return Ok(PackageSpec::Local {
                path: PathBuf::from(text),
            });
        }
        let (base, git_ref) = match text.split_once('@') {
            Some((base, git_ref)) => (base, Some(parse_git_ref(git_ref)?)),
            None => (text, None),
        };
        let Some((owner, repo)) = base.split_once('/') else {
            return Err(format!(
                "invalid package spec '{text}': expected 'owner/repo[@tag|@commit]' or a local path"
            ));
        };
        if !is_github_name(owner) || !is_github_name(repo) {
            return Err(format!(
                "invalid package spec '{text}': bad owner or repo name"
            ));
        }
        Ok(PackageSpec::GitHub {
            owner: owner.to_string(),
            repo: repo.to_string(),
            git_ref,
        })
    }

    /// The version this spec demands, when it names one: a
    /// semver tag (`@1.2.3` or `@v1.2.3`). A commit ref or no
    /// ref demands no particular version — the manifest's
    /// version is authoritative then.
    pub fn demanded_version(&self) -> Option<SemVer> {
        match self {
            PackageSpec::GitHub {
                git_ref: Some(GitRef::Tag(tag)),
                ..
            } => SemVer::parse_tag(tag),
            _ => None,
        }
    }

    /// The `source` string recorded in the lockfile.
    pub fn source_string(&self) -> String {
        match self {
            PackageSpec::GitHub { owner, repo, .. } => format!("github:{owner}/{repo}"),
            PackageSpec::Local { path } => format!("local:{}", path.display()),
        }
    }
}

fn parse_git_ref(text: &str) -> Result<GitRef, String> {
    if text.is_empty() {
        return Err("empty @ref in package spec".to_string());
    }
    if text.len() >= 7 && text.len() <= 40 && text.chars().all(|c| c.is_ascii_hexdigit()) {
        Ok(GitRef::Commit(text.to_string()))
    } else {
        Ok(GitRef::Tag(text.to_string()))
    }
}

fn is_github_name(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
}

/// The clone URL of a GitHub repository.
pub fn github_url(owner: &str, repo: &str) -> String {
    format!("https://github.com/{owner}/{repo}.git")
}

/// Where a package is installed: the global store or the
/// project-local `gg_packages/`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Location {
    Global,
    Local,
}

impl Location {
    fn as_str(&self) -> &'static str {
        match self {
            Location::Global => "global",
            Location::Local => "local",
        }
    }
}

/// One entry of `gg.lock`: where a package came from, which
/// version is installed, and the exact thing it resolved to
/// (a commit hash for GitHub, the canonical path for local).
#[derive(Debug, Clone)]
pub struct LockEntry {
    pub source: String,
    pub version: SemVer,
    pub resolved: String,
    pub location: Location,
}

/// The lockfile: installed packages by name, in alphabetical
/// order (a `BTreeMap`, so the file is deterministic).
#[derive(Debug, Clone, Default)]
pub struct Lockfile {
    pub packages: BTreeMap<String, LockEntry>,
}

/// The lockfile of the current project: `gg.lock` in the
/// working directory.
pub fn lockfile_path() -> PathBuf {
    std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join("gg.lock")
}

/// Read a lockfile; a missing file is an empty lockfile (a
/// fresh project), a corrupt one is an error naming the file.
pub fn read_lockfile(path: &Path) -> Result<Lockfile, String> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Lockfile::default()),
        Err(e) => return Err(format!("cannot read {}: {e}", path.display())),
    };
    parse_lockfile(&text).map_err(|e| format!("{}: {e}", path.display()))
}

fn parse_lockfile(text: &str) -> Result<Lockfile, String> {
    let value = json::parse_text(text).map_err(|e| e.to_string())?;
    let Value::Dict(root) = value else {
        return Err("lockfile must be a JSON object".to_string());
    };
    let root = root.borrow();
    let Some(Value::Dict(packages)) = root.get(&DictKey::Str("packages".to_string())) else {
        return Err("lockfile: missing 'packages' object".to_string());
    };
    let packages = packages.borrow();
    let mut lock = Lockfile::default();
    for key in packages.keys() {
        let DictKey::Str(name) = key else {
            return Err("lockfile: package names must be strings".to_string());
        };
        let Some(Value::Dict(entry)) = packages.get(&DictKey::Str(name.clone())) else {
            return Err(format!("lockfile: package '{name}' must be an object"));
        };
        let entry = entry.borrow();
        let source = lock_str(&entry, &name, "source")?;
        let version_text = lock_str(&entry, &name, "version")?;
        let version =
            SemVer::parse(&version_text).map_err(|e| format!("lockfile: package '{name}': {e}"))?;
        let resolved = lock_str(&entry, &name, "resolved")?;
        let location = match entry.get(&DictKey::Str("location".to_string())) {
            None => Location::Global,
            Some(Value::Str(value)) if value == "global" => Location::Global,
            Some(Value::Str(value)) if value == "local" => Location::Local,
            Some(Value::Str(value)) => {
                return Err(format!(
                    "lockfile: package '{name}': unknown location '{value}'"
                ));
            }
            Some(_) => {
                return Err(format!(
                    "lockfile: package '{name}': 'location' must be a string"
                ));
            }
        };
        lock.packages.insert(
            name,
            LockEntry {
                source,
                version,
                resolved,
                location,
            },
        );
    }
    Ok(lock)
}

fn lock_str(entry: &DictData, name: &str, key: &str) -> Result<String, String> {
    match entry.get(&DictKey::Str(key.to_string())) {
        Some(Value::Str(value)) => Ok(value.clone()),
        Some(_) => Err(format!(
            "lockfile: package '{name}': '{key}' must be a string"
        )),
        None => Err(format!("lockfile: package '{name}': missing '{key}'")),
    }
}

/// Write a lockfile (pretty JSON, 2 spaces, keys in
/// alphabetical order). The lockfile is written through the
/// `json` module's own renderer, so its escaping is the same
/// as `json.stringify`.
pub fn write_lockfile(path: &Path, lock: &Lockfile) -> Result<(), String> {
    let mut root = DictData::new();
    let mut packages = DictData::new();
    for (name, entry) in &lock.packages {
        let mut item = DictData::new();
        item.insert(
            DictKey::Str("source".to_string()),
            Value::Str(entry.source.clone()),
        );
        item.insert(
            DictKey::Str("version".to_string()),
            Value::Str(entry.version.to_string()),
        );
        item.insert(
            DictKey::Str("resolved".to_string()),
            Value::Str(entry.resolved.clone()),
        );
        item.insert(
            DictKey::Str("location".to_string()),
            Value::Str(entry.location.as_str().to_string()),
        );
        packages.insert(
            DictKey::Str(name.clone()),
            Value::Dict(Rc::new(RefCell::new(item))),
        );
    }
    root.insert(
        DictKey::Str("packages".to_string()),
        Value::Dict(Rc::new(RefCell::new(packages))),
    );
    let value = Value::Dict(Rc::new(RefCell::new(root)));
    let text = json::stringify_value(&value, 2).map_err(|e| e.to_string())?;
    std::fs::write(path, format!("{text}\n"))
        .map_err(|e| format!("cannot write {}: {e}", path.display()))
}

/// The global package store: `~/.gg/packages`.
pub fn global_packages_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".gg").join("packages")
}

/// The package roots `import` searches, in order: the
/// project-local `gg_packages/` in the working directory, then
/// the global store. A root that does not exist is simply
/// skipped at import time.
pub fn default_pkg_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Ok(cwd) = std::env::current_dir() {
        roots.push(cwd.join("gg_packages"));
    }
    roots.push(global_packages_dir());
    roots
}

/// The entry file of an installed package in one root, or
/// `None`: `<root>/<name>/<version>/<entry>` where the version
/// is the lockfile's pin when it lists the package, else the
/// newest installed version. The manifest must exist and its
/// name must match the directory (a package is never found
/// under another name).
pub fn resolve_package_entry(name: &str, root: &Path, lock: Option<&Lockfile>) -> Option<PathBuf> {
    let pkg_dir = root.join(name);
    if !pkg_dir.is_dir() {
        return None;
    }
    let version = match lock.and_then(|lock| lock.packages.get(name)) {
        Some(entry) => entry.version,
        None => highest_installed_version(&pkg_dir)?,
    };
    let dir = pkg_dir.join(version.to_string());
    let manifest_text = std::fs::read_to_string(dir.join("gg.toml")).ok()?;
    let manifest = parse_manifest(&manifest_text).ok()?;
    if manifest.name != name {
        return None;
    }
    let entry = dir.join(&manifest.entry);
    if entry.is_file() { Some(entry) } else { None }
}

fn highest_installed_version(pkg_dir: &Path) -> Option<SemVer> {
    let mut best: Option<SemVer> = None;
    let entries = std::fs::read_dir(pkg_dir).ok()?;
    for entry in entries.flatten() {
        if !entry.path().is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        if let Ok(version) = SemVer::parse(&name)
            && best.is_none_or(|current| version > current)
        {
            best = Some(version);
        }
    }
    best
}

/// Options for an install: where it lands and which project
/// the lockfile belongs to.
#[derive(Debug, Clone)]
pub struct InstallOpts {
    /// Install into the project's `gg_packages/` instead of
    /// the global store.
    pub local: bool,
    /// The project directory: the lockfile lives here, and a
    /// local install lands in `<project_dir>/gg_packages/`.
    pub project_dir: PathBuf,
    /// The global store (`~/.gg/packages`).
    pub global_root: PathBuf,
}

/// One package installed by an `install` run: what it is,
/// where it landed, and whether it was already there.
#[derive(Debug, Clone)]
pub struct InstallReport {
    pub name: String,
    pub version: SemVer,
    pub dir: PathBuf,
    pub already_installed: bool,
    pub source: String,
    pub resolved: String,
    pub location: Location,
}

/// A fetched package: its directory, its manifest, and what
/// it resolved to (a commit hash for GitHub, the canonical
/// path for a local directory). A GitHub fetch owns a
/// temporary directory, removed when the `Fetched` value is
/// dropped; a local fetch borrows the source directory.
struct Fetched {
    dir: PathBuf,
    manifest: Manifest,
    resolved: String,
    _temp: Option<TempDir>,
}

/// A temporary directory, removed when dropped.
struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(tag: &str) -> TempDir {
        let path = std::env::temp_dir().join(format!(
            "gg-pkg-{}-{tag}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let _ = std::fs::remove_dir_all(&path);
        TempDir { path }
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// The state of one `install` run: which versions are already
/// spoken for (a second, different version demanded for the
/// same package is a conflict — v1 has no resolver), which
/// packages are being installed right now (a cycle), and
/// which are done (their dependencies were already walked).
struct Installer {
    opts: InstallOpts,
    wanted: HashMap<String, (SemVer, String)>,
    visiting: Vec<String>,
    done: HashSet<String>,
    reports: Vec<InstallReport>,
}

impl Installer {
    fn root(&self, location: Location) -> PathBuf {
        match location {
            Location::Global => self.opts.global_root.clone(),
            Location::Local => self.opts.project_dir.join("gg_packages"),
        }
    }

    /// Install one spec and its dependencies. `wanted_by`
    /// names who asked for it (the command line, or the
    /// package that depends on it) for conflict messages.
    /// A relative local spec resolves against `base_dir`:
    /// the project directory for a command-line spec, the
    /// depending package's directory for a dependency.
    fn install_spec(
        &mut self,
        spec: &PackageSpec,
        wanted_by: &str,
        base_dir: &Path,
    ) -> Result<(), String> {
        let spec = match spec {
            PackageSpec::Local { path } if path.is_relative() => PackageSpec::Local {
                path: base_dir.join(path),
            },
            other => other.clone(),
        };
        let fetched = fetch(&spec)?;
        let name = fetched.manifest.name.clone();
        let version = fetched.manifest.version;
        // A spec that names a version must get exactly that
        // version: the manifest has to agree with its tag.
        if let Some(demanded) = spec.demanded_version()
            && demanded != version
        {
            return Err(format!(
                "package '{name}' is version {version} but '{wanted_by}' asked for {demanded}"
            ));
        }
        if let Some((other, by)) = self.wanted.get(&name) {
            if *other != version {
                return Err(format!(
                    "version conflict for package '{name}': '{by}' wants {other} but '{wanted_by}' wants {version} (GG v1 has no version resolver: pick one version)"
                ));
            }
            if self.done.contains(&name) {
                return Ok(());
            }
        } else {
            self.wanted
                .insert(name.clone(), (version, wanted_by.to_string()));
        }
        if self.visiting.contains(&name) {
            let mut chain = self.visiting.clone();
            chain.push(name.clone());
            return Err(format!("dependency cycle: {}", chain.join(" -> ")));
        }
        let location = if self.opts.local {
            Location::Local
        } else {
            Location::Global
        };
        let target = self.root(location).join(&name).join(version.to_string());
        let already = target.join("gg.toml").is_file();
        if !already {
            std::fs::create_dir_all(&target)
                .map_err(|e| format!("cannot create {}: {e}", target.display()))?;
            copy_package_files(&fetched.dir, &target)?;
        }
        self.reports.push(InstallReport {
            name: name.clone(),
            version,
            dir: target,
            already_installed: already,
            source: spec.source_string(),
            resolved: fetched.resolved.clone(),
            location,
        });
        self.visiting.push(name.clone());
        for dep in &fetched.manifest.dependencies {
            self.install_spec(dep, &name, &fetched.dir)?;
        }
        self.visiting.pop();
        self.done.insert(name);
        Ok(())
    }
}

/// Install a package (and its dependencies) and update the
/// project's lockfile. Returns one report per package
/// installed, top-level first. An already-installed package
/// is reported with `already_installed` and its dependencies
/// are still walked.
pub fn install(spec: &PackageSpec, opts: &InstallOpts) -> Result<Vec<InstallReport>, String> {
    let mut installer = Installer {
        opts: opts.clone(),
        wanted: HashMap::new(),
        visiting: Vec::new(),
        done: HashSet::new(),
        reports: Vec::new(),
    };
    installer.install_spec(spec, "command line", &opts.project_dir)?;
    update_lockfile(&opts.project_dir.join("gg.lock"), &installer.reports)?;
    Ok(installer.reports)
}

/// Install several specs in one run (one `wanted` map, so a
/// conflict between them is caught).
pub fn install_many(
    specs: &[PackageSpec],
    opts: &InstallOpts,
) -> Result<Vec<InstallReport>, String> {
    let mut installer = Installer {
        opts: opts.clone(),
        wanted: HashMap::new(),
        visiting: Vec::new(),
        done: HashSet::new(),
        reports: Vec::new(),
    };
    for spec in specs {
        installer.install_spec(spec, "command line", &opts.project_dir)?;
    }
    update_lockfile(&opts.project_dir.join("gg.lock"), &installer.reports)?;
    Ok(installer.reports)
}

/// Reinstall every package in the project's lockfile, exactly
/// at the pinned versions (`npm ci`, simplified): the
/// lockfile is the source of truth, dependencies are not
/// re-walked (they are in the lockfile already). A package
/// whose directory is already there is skipped.
pub fn install_from_lockfile(opts: &InstallOpts) -> Result<Vec<InstallReport>, String> {
    let lock_path = opts.project_dir.join("gg.lock");
    let lock = read_lockfile(&lock_path)?;
    if lock.packages.is_empty() {
        return Err(format!(
            "no packages in {} (nothing to install)",
            lock_path.display()
        ));
    }
    let mut reports = Vec::new();
    for (name, entry) in &lock.packages {
        let root = match entry.location {
            Location::Global => opts.global_root.clone(),
            Location::Local => opts.project_dir.join("gg_packages"),
        };
        let target = root.join(name).join(entry.version.to_string());
        if target.join("gg.toml").is_file() {
            reports.push(InstallReport {
                name: name.clone(),
                version: entry.version,
                dir: target,
                already_installed: true,
                source: entry.source.clone(),
                resolved: entry.resolved.clone(),
                location: entry.location,
            });
            continue;
        }
        let spec = spec_from_lock_entry(entry)?;
        let fetched = fetch(&spec)?;
        if fetched.manifest.name != *name {
            return Err(format!(
                "lockfile out of date for '{name}': the source now provides '{}' (run 'gg install {}' again)",
                fetched.manifest.name, entry.source
            ));
        }
        if fetched.manifest.version != entry.version {
            return Err(format!(
                "lockfile out of date for '{name}': the source now provides version {} but the lockfile pins {} (run 'gg install {}' again)",
                fetched.manifest.version, entry.version, entry.source
            ));
        }
        std::fs::create_dir_all(&target)
            .map_err(|e| format!("cannot create {}: {e}", target.display()))?;
        copy_package_files(&fetched.dir, &target)?;
        reports.push(InstallReport {
            name: name.clone(),
            version: entry.version,
            dir: target,
            already_installed: false,
            source: entry.source.clone(),
            resolved: entry.resolved.clone(),
            location: entry.location,
        });
    }
    Ok(reports)
}

fn spec_from_lock_entry(entry: &LockEntry) -> Result<PackageSpec, String> {
    match entry.source.strip_prefix("github:") {
        Some(rest) => {
            let (owner, repo) = rest
                .split_once('/')
                .ok_or_else(|| format!("lockfile: bad github source '{}'", entry.source))?;
            let git_ref = if entry.resolved.len() == 40
                && entry.resolved.chars().all(|c| c.is_ascii_hexdigit())
            {
                Some(GitRef::Commit(entry.resolved.clone()))
            } else {
                Some(GitRef::Tag(entry.resolved.clone()))
            };
            Ok(PackageSpec::GitHub {
                owner: owner.to_string(),
                repo: repo.to_string(),
                git_ref,
            })
        }
        None => match entry.source.strip_prefix("local:") {
            Some(path) => Ok(PackageSpec::Local {
                path: PathBuf::from(path),
            }),
            None => Err(format!("lockfile: bad source '{}'", entry.source)),
        },
    }
}

fn update_lockfile(path: &Path, reports: &[InstallReport]) -> Result<(), String> {
    let mut lock = read_lockfile(path)?;
    for report in reports {
        lock.packages.insert(
            report.name.clone(),
            LockEntry {
                source: report.source.clone(),
                version: report.version,
                resolved: report.resolved.clone(),
                location: report.location,
            },
        );
    }
    write_lockfile(path, &lock)
}

/// Fetch a package: a local directory is used as-is, a GitHub
/// repository is cloned into a temporary directory. Returns
/// the manifest and what the package resolved to.
fn fetch(spec: &PackageSpec) -> Result<Fetched, String> {
    match spec {
        PackageSpec::Local { path } => {
            if !path.is_dir() {
                return Err(format!(
                    "local package '{}' does not exist (not a directory)",
                    path.display()
                ));
            }
            let manifest_path = path.join("gg.toml");
            let text = std::fs::read_to_string(&manifest_path).map_err(|_| {
                format!(
                    "local package '{}': no gg.toml found (not a GG package)",
                    path.display()
                )
            })?;
            let manifest = parse_manifest(&text)?;
            let resolved = std::fs::canonicalize(path)
                .map(|p| p.display().to_string())
                .unwrap_or_else(|_| path.display().to_string());
            Ok(Fetched {
                dir: path.clone(),
                manifest,
                resolved,
                _temp: None,
            })
        }
        PackageSpec::GitHub {
            owner,
            repo,
            git_ref,
        } => {
            let url = github_url(owner, repo);
            let temp = TempDir::new(&format!("{owner}-{repo}"));
            clone_repo(&url, git_ref, &temp.path)?;
            let manifest_path = temp.path.join("gg.toml");
            let text = std::fs::read_to_string(&manifest_path).map_err(|_| {
                format!("github:{owner}/{repo}: no gg.toml found (not a GG package)")
            })?;
            let manifest = parse_manifest(&text)?;
            let resolved = git_head_sha(&temp.path).unwrap_or_else(|| match git_ref {
                Some(GitRef::Tag(tag)) => tag.clone(),
                Some(GitRef::Commit(sha)) => sha.clone(),
                None => "HEAD".to_string(),
            });
            Ok(Fetched {
                dir: temp.path.clone(),
                manifest,
                resolved,
                _temp: Some(temp),
            })
        }
    }
}

/// Clone a repository with `git`: `--depth 1` for a tag or the
/// default branch, a full clone + checkout for a commit (a
/// commit may not be at the tip). The `url` may also be a
/// local path (used by the tests, so cloning is testable
/// offline).
fn clone_repo(url: &str, git_ref: &Option<GitRef>, dest: &Path) -> Result<(), String> {
    let dest_arg = dest.display().to_string();
    match git_ref {
        Some(GitRef::Tag(tag)) => {
            run_git(&[
                "clone".to_string(),
                "--depth".to_string(),
                "1".to_string(),
                "--branch".to_string(),
                tag.clone(),
                url.to_string(),
                dest_arg,
            ])?;
        }
        Some(GitRef::Commit(sha)) => {
            run_git(&["clone".to_string(), url.to_string(), dest_arg.clone()])?;
            run_git(&[
                "-C".to_string(),
                dest_arg,
                "checkout".to_string(),
                sha.clone(),
            ])?;
        }
        None => {
            run_git(&[
                "clone".to_string(),
                "--depth".to_string(),
                "1".to_string(),
                url.to_string(),
                dest_arg,
            ])?;
        }
    }
    Ok(())
}

/// Run `git` with the given arguments. A missing `git`
/// binary, and a failing command, are both clear errors
/// (the last lines of git's stderr are included).
fn run_git(args: &[String]) -> Result<(), String> {
    let output = ProcCommand::new("git").args(args).output().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            "git is not installed: 'gg install' needs git to install from GitHub (install git, or use a local path)"
                .to_string()
        } else {
            format!("cannot run git: {e}")
        }
    })?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let tail: Vec<&str> = stderr.lines().rev().take(3).collect();
        let tail: Vec<&str> = tail.into_iter().rev().collect();
        return Err(format!("git {} failed: {}", args.join(" "), tail.join(" ")));
    }
    Ok(())
}

fn git_head_sha(dir: &Path) -> Option<String> {
    let output = ProcCommand::new("git")
        .args(["-C", &dir.display().to_string(), "rev-parse", "HEAD"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let sha = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if sha.is_empty() { None } else { Some(sha) }
}

/// Copy a package's files: every `.gg` file (at any depth)
/// and the root `gg.toml`. `.git` and `target` directories
/// are never copied.
fn copy_package_files(src: &Path, dst: &Path) -> Result<(), String> {
    let mut stack = vec![src.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries =
            std::fs::read_dir(&dir).map_err(|e| format!("cannot read {}: {e}", dir.display()))?;
        for entry in entries {
            let entry = entry.map_err(|e| format!("cannot read {}: {e}", dir.display()))?;
            let path = entry.path();
            let name = entry.file_name();
            if path.is_dir() {
                if name == ".git" || name == "target" {
                    continue;
                }
                stack.push(path);
            } else if path.extension().is_some_and(|ext| ext == "gg") || name == "gg.toml" {
                let rel = path
                    .strip_prefix(src)
                    .map_err(|_| "internal error: file outside package".to_string())?;
                let dest = dst.join(rel);
                if let Some(parent) = dest.parent() {
                    std::fs::create_dir_all(parent)
                        .map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
                }
                std::fs::copy(&path, &dest).map_err(|e| {
                    format!("cannot copy {} to {}: {e}", path.display(), dest.display())
                })?;
            }
        }
    }
    Ok(())
}

/// One installed package: its name, version and where it
/// lives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledPackage {
    pub name: String,
    pub version: SemVer,
    pub location: Location,
}

/// Every installed package (global store first by location
/// order in the list: local, then global), sorted by name
/// and version.
pub fn list_installed(global_root: &Path, local_root: &Path) -> Vec<InstalledPackage> {
    let mut out = Vec::new();
    for (root, location) in [
        (local_root, Location::Local),
        (global_root, Location::Global),
    ] {
        let Ok(entries) = std::fs::read_dir(root) else {
            continue;
        };
        for pkg in entries.flatten() {
            if !pkg.path().is_dir() {
                continue;
            }
            let name = pkg.file_name().to_string_lossy().into_owned();
            let Ok(versions) = std::fs::read_dir(pkg.path()) else {
                continue;
            };
            for version_dir in versions.flatten() {
                if !version_dir.path().is_dir() {
                    continue;
                }
                let version_name = version_dir.file_name().to_string_lossy().into_owned();
                if let Ok(version) = SemVer::parse(&version_name)
                    && version_dir.path().join("gg.toml").is_file()
                {
                    out.push(InstalledPackage {
                        name: name.clone(),
                        version,
                        location,
                    });
                }
            }
        }
    }
    out.sort_by_key(|pkg| (pkg.name.clone(), pkg.version));
    out
}

/// Remove every installed version of a package (both roots)
/// and drop it from the project's lockfile. Returns the
/// removed directories; an error when nothing was installed.
pub fn remove_package(
    name: &str,
    global_root: &Path,
    local_root: &Path,
) -> Result<Vec<PathBuf>, String> {
    if !is_valid_package_name(name) {
        return Err(format!("invalid package name '{name}'"));
    }
    let mut removed = Vec::new();
    for root in [local_root, global_root] {
        let dir = root.join(name);
        if dir.is_dir() {
            std::fs::remove_dir_all(&dir)
                .map_err(|e| format!("cannot remove {}: {e}", dir.display()))?;
            removed.push(dir);
        }
    }
    if removed.is_empty() {
        return Err(format!("package '{name}' is not installed"));
    }
    let lock_path = lockfile_path();
    if let Ok(mut lock) = read_lockfile(&lock_path)
        && lock.packages.remove(name).is_some()
    {
        let _ = write_lockfile(&lock_path, &lock);
    }
    Ok(removed)
}

/// True when `name` is also a built-in module (an embedded
/// stdlib module or a native one): `import <name>` loads the
/// built-in, not an installed package with that name.
pub fn shadows_builtin(name: &str) -> bool {
    crate::stdlib::get(name).is_some() || crate::native::build(name).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interp::Interpreter;
    use crate::vm::Backend;

    /// A fresh scratch directory for a package test (unique per
    /// test tag, removed when the test is done).
    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("gg-test-pkg-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Write a fixture package directory: `gg.toml` + `lib.gg`.
    fn fixture_pkg(dir: &Path, name: &str, version: &str, deps: &[&str], lib: &str) {
        std::fs::create_dir_all(dir).unwrap();
        let deps_toml = if deps.is_empty() {
            String::new()
        } else {
            format!(
                "dependencies = [{}]\n",
                deps.iter()
                    .map(|d| format!("\"{d}\""))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        };
        std::fs::write(
            dir.join("gg.toml"),
            format!("name = \"{name}\"\nversion = \"{version}\"\n{deps_toml}"),
        )
        .unwrap();
        std::fs::write(dir.join("lib.gg"), lib).unwrap();
    }

    fn install_opts(project_dir: &Path, global_root: &Path) -> InstallOpts {
        InstallOpts {
            local: false,
            project_dir: project_dir.to_path_buf(),
            global_root: global_root.to_path_buf(),
        }
    }

    /// Run GG `source` with the given package roots on one
    /// backend, returning its printed output.
    fn run_with_pkg_roots(
        source: &str,
        pkg_roots: Vec<PathBuf>,
        backend: Backend,
    ) -> Result<String, String> {
        let tokens = gg_lexer::lex(source).map_err(|e| e.to_string())?;
        let program = gg_parser::parse(&tokens).map_err(|e| e.to_string())?;
        let mut interp = Interpreter::new();
        interp.set_backend(backend);
        interp.set_pkg_roots(pkg_roots);
        let mut input = std::io::Cursor::new(Vec::new());
        let mut out = Vec::new();
        interp
            .run(&program, &mut input, &mut out)
            .map_err(|e| e.to_string())?;
        Ok(String::from_utf8(out).unwrap())
    }

    #[test]
    fn semver_parses_and_orders() {
        assert_eq!(
            SemVer::parse("1.2.3").unwrap(),
            SemVer {
                major: 1,
                minor: 2,
                patch: 3
            }
        );
        assert_eq!(SemVer::parse("0.0.0").unwrap().to_string(), "0.0.0");
        assert!(SemVer::parse("1.2").is_err());
        assert!(SemVer::parse("1.2.3.4").is_err());
        assert!(SemVer::parse("1.x.3").is_err());
        assert!(SemVer::parse("v1.2.3").is_err());
        assert!(SemVer::parse("1.2.3-beta").is_err());
        assert!(SemVer::parse("").is_err());
        assert!(SemVer::parse("1.2.3").unwrap() < SemVer::parse("1.2.4").unwrap());
        assert!(SemVer::parse("1.10.0").unwrap() > SemVer::parse("1.9.9").unwrap());
        assert_eq!(
            SemVer::parse_tag("v1.2.3"),
            Some(SemVer::parse("1.2.3").unwrap())
        );
        assert_eq!(
            SemVer::parse_tag("1.2.3"),
            Some(SemVer::parse("1.2.3").unwrap())
        );
        assert_eq!(SemVer::parse_tag("latest"), None);
    }

    #[test]
    fn package_names_must_be_identifiers() {
        assert!(is_valid_package_name("ciao"));
        assert!(is_valid_package_name("_x1"));
        assert!(!is_valid_package_name(""));
        assert!(!is_valid_package_name("1ciao"));
        assert!(!is_valid_package_name("ciao-pkg"));
        assert!(!is_valid_package_name("ciao.pkg"));
        assert!(!is_valid_package_name("ciao pkg"));
    }

    #[test]
    fn manifest_parses_full_and_minimal() {
        let full = "name = \"ciao\"\nversion = \"1.2.3\"\ndescription = \"Saluti\"\nentry = \"main.gg\"\ndependencies = [\"owner/altro@1.0.0\", \"./locale\"]\n";
        let manifest = parse_manifest(full).unwrap();
        assert_eq!(manifest.name, "ciao");
        assert_eq!(manifest.version, SemVer::parse("1.2.3").unwrap());
        assert_eq!(manifest.description.as_deref(), Some("Saluti"));
        assert_eq!(manifest.entry, "main.gg");
        assert_eq!(manifest.dependencies.len(), 2);
        let minimal = "name = \"ciao\"\nversion = \"0.1.0\"\n";
        let manifest = parse_manifest(minimal).unwrap();
        assert_eq!(manifest.entry, "lib.gg");
        assert!(manifest.description.is_none());
        assert!(manifest.dependencies.is_empty());
    }

    #[test]
    fn manifest_errors_are_clear() {
        assert!(
            parse_manifest("version = \"1.0.0\"\n")
                .unwrap_err()
                .contains("missing required field 'name'")
        );
        assert!(
            parse_manifest("name = \"ciao\"\n")
                .unwrap_err()
                .contains("missing required field 'version'")
        );
        assert!(
            parse_manifest("name = \"ciao\"\nversion = \"1.0\"\n")
                .unwrap_err()
                .contains("invalid version")
        );
        assert!(
            parse_manifest("name = \"ciao-pkg\"\nversion = \"1.0.0\"\n")
                .unwrap_err()
                .contains("invalid package name")
        );
        assert!(
            parse_manifest("name = \"ciao\"\nversion = \"1.0.0\"\nentry = \"../fuori.gg\"\n")
                .unwrap_err()
                .contains("invalid entry")
        );
        assert!(
            parse_manifest("name = \"ciao\"\nversion = \"1.0.0\"\nentry = \"main.txt\"\n")
                .unwrap_err()
                .contains("invalid entry")
        );
        assert!(
            parse_manifest("name = \"ciao\"\nversion = \"1.0.0\"\ndependencies = \"owner/x\"\n")
                .unwrap_err()
                .contains("list of strings")
        );
        // A TOML syntax error names the position.
        let err = parse_manifest("name = \"ciao\"\nversion = \n").unwrap_err();
        assert!(err.contains("gg.toml:"), "{err}");
        assert!(err.contains("line"), "{err}");
    }

    #[test]
    fn spec_parses_github_and_local() {
        assert_eq!(
            PackageSpec::parse("owner/repo").unwrap(),
            PackageSpec::GitHub {
                owner: "owner".to_string(),
                repo: "repo".to_string(),
                git_ref: None,
            }
        );
        assert_eq!(
            PackageSpec::parse("owner/repo@v1.2.3").unwrap(),
            PackageSpec::GitHub {
                owner: "owner".to_string(),
                repo: "repo".to_string(),
                git_ref: Some(GitRef::Tag("v1.2.3".to_string())),
            }
        );
        assert_eq!(
            PackageSpec::parse("owner/repo@abcdef0123456789abcdef0123456789abcdef01").unwrap(),
            PackageSpec::GitHub {
                owner: "owner".to_string(),
                repo: "repo".to_string(),
                git_ref: Some(GitRef::Commit(
                    "abcdef0123456789abcdef0123456789abcdef01".to_string()
                )),
            }
        );
        assert_eq!(
            PackageSpec::parse("./pacchetto").unwrap(),
            PackageSpec::Local {
                path: PathBuf::from("./pacchetto"),
            }
        );
        assert_eq!(
            PackageSpec::parse("/tmp/pacchetto").unwrap(),
            PackageSpec::Local {
                path: PathBuf::from("/tmp/pacchetto"),
            }
        );
        assert!(PackageSpec::parse("").is_err());
        assert!(PackageSpec::parse("solo-nome").is_err());
        assert!(PackageSpec::parse("owner/").is_err());
        assert!(PackageSpec::parse("owner/repo@").is_err());
        // A version tag demands that version; a commit does not.
        assert_eq!(
            PackageSpec::parse("o/r@1.2.3").unwrap().demanded_version(),
            Some(SemVer::parse("1.2.3").unwrap())
        );
        assert_eq!(
            PackageSpec::parse("o/r@v2.0.0").unwrap().demanded_version(),
            Some(SemVer::parse("2.0.0").unwrap())
        );
        assert_eq!(
            PackageSpec::parse("o/r@abcdef0")
                .unwrap()
                .demanded_version(),
            None
        );
        assert_eq!(PackageSpec::parse("o/r").unwrap().demanded_version(), None);
        assert_eq!(github_url("o", "r"), "https://github.com/o/r.git");
    }

    #[test]
    fn lockfile_round_trips_and_missing_is_empty() {
        let dir = temp_dir("lock");
        let path = dir.join("gg.lock");
        assert!(read_lockfile(&path).unwrap().packages.is_empty());
        let mut lock = Lockfile::default();
        lock.packages.insert(
            "ciao".to_string(),
            LockEntry {
                source: "github:o/r".to_string(),
                version: SemVer::parse("1.2.3").unwrap(),
                resolved: "abc123".to_string(),
                location: Location::Global,
            },
        );
        lock.packages.insert(
            "altro".to_string(),
            LockEntry {
                source: "local:/tmp/altro".to_string(),
                version: SemVer::parse("0.1.0").unwrap(),
                resolved: "/tmp/altro".to_string(),
                location: Location::Local,
            },
        );
        write_lockfile(&path, &lock).unwrap();
        let back = read_lockfile(&path).unwrap();
        assert_eq!(back.packages.len(), 2);
        assert_eq!(back.packages["ciao"].version.to_string(), "1.2.3");
        assert_eq!(back.packages["ciao"].resolved, "abc123");
        assert_eq!(back.packages["ciao"].location, Location::Global);
        assert_eq!(back.packages["altro"].location, Location::Local);
        // The file is JSON with the expected shape.
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("\"packages\""), "{text}");
        assert!(text.contains("\"github:o/r\""), "{text}");
        // Corrupt JSON is an error naming the file.
        std::fs::write(&path, "{not json").unwrap();
        let err = read_lockfile(&path).unwrap_err();
        assert!(err.contains("gg.lock"), "{err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn install_from_local_path_and_import_on_both_backends() {
        let dir = temp_dir("install-local");
        let project = dir.join("proj");
        let global = dir.join("home").join(".gg").join("packages");
        std::fs::create_dir_all(&project).unwrap();
        let src = dir.join("ciao-src");
        fixture_pkg(
            &src,
            "ciao",
            "1.0.0",
            &[],
            "fn saluta(nome)\n    return \"Ciao \" + nome + \"!\"\nend\nfn doppio(x)\n    return x * 2\nend\n",
        );
        let opts = install_opts(&project, &global);
        let reports = install(&PackageSpec::Local { path: src }, &opts).unwrap();
        assert_eq!(reports.len(), 1);
        assert_eq!(reports[0].name, "ciao");
        assert_eq!(reports[0].version.to_string(), "1.0.0");
        assert!(!reports[0].already_installed);
        assert!(global.join("ciao").join("1.0.0").join("lib.gg").is_file());
        assert!(global.join("ciao").join("1.0.0").join("gg.toml").is_file());
        // The lockfile records the install.
        let lock = read_lockfile(&project.join("gg.lock")).unwrap();
        assert_eq!(lock.packages["ciao"].version.to_string(), "1.0.0");
        assert!(lock.packages["ciao"].source.starts_with("local:"));
        // Installing again reports it as already installed.
        let reports = install(
            &PackageSpec::Local {
                path: dir.join("ciao-src"),
            },
            &opts,
        )
        .unwrap();
        assert_eq!(reports.len(), 1);
        assert!(reports[0].already_installed);
        // And a GG program imports it, on both backends.
        let roots = vec![global.clone()];
        let src = "import ciao\nprint ciao.saluta(\"Giuliano\")\nprint ciao.doppio(21)\n";
        assert_eq!(
            run_with_pkg_roots(src, roots.clone(), Backend::Vm).unwrap(),
            "Ciao Giuliano!\n42\n"
        );
        assert_eq!(
            run_with_pkg_roots(src, roots, Backend::Tree).unwrap(),
            "Ciao Giuliano!\n42\n"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn install_resolves_highest_version_and_lockfile_pin() {
        let dir = temp_dir("versions");
        let project = dir.join("proj");
        let global = dir.join("home").join(".gg").join("packages");
        std::fs::create_dir_all(&project).unwrap();
        let opts = install_opts(&project, &global);
        fixture_pkg(
            &dir.join("v1"),
            "ciao",
            "1.0.0",
            &[],
            "fn v()\n    return 1\nend\n",
        );
        fixture_pkg(
            &dir.join("v2"),
            "ciao",
            "2.0.0",
            &[],
            "fn v()\n    return 2\nend\n",
        );
        install(
            &PackageSpec::Local {
                path: dir.join("v1"),
            },
            &opts,
        )
        .unwrap();
        install(
            &PackageSpec::Local {
                path: dir.join("v2"),
            },
            &opts,
        )
        .unwrap();
        // No lockfile pin: the newest version wins.
        let entry = resolve_package_entry("ciao", &global, None).unwrap();
        assert!(entry.ends_with("2.0.0/lib.gg"), "{}", entry.display());
        // A lockfile pinning 1.0.0 wins instead.
        let mut lock = Lockfile::default();
        lock.packages.insert(
            "ciao".to_string(),
            LockEntry {
                source: "local:/x".to_string(),
                version: SemVer::parse("1.0.0").unwrap(),
                resolved: "/x".to_string(),
                location: Location::Global,
            },
        );
        let entry = resolve_package_entry("ciao", &global, Some(&lock)).unwrap();
        assert!(entry.ends_with("1.0.0/lib.gg"), "{}", entry.display());
        // A pin for a version that is not installed resolves to
        // nothing (the next root is tried instead).
        lock.packages.get_mut("ciao").unwrap().version = SemVer::parse("9.9.9").unwrap();
        assert!(resolve_package_entry("ciao", &global, Some(&lock)).is_none());
        assert!(resolve_package_entry("manca", &global, None).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn install_walks_dependencies_and_detects_cycles() {
        let dir = temp_dir("deps");
        let project = dir.join("proj");
        let global = dir.join("home").join(".gg").join("packages");
        std::fs::create_dir_all(&project).unwrap();
        let opts = install_opts(&project, &global);
        // a -> b -> a: a cycle. A relative dependency resolves
        // against the depending package's own directory (npm
        // rule), so `b` lives inside `a` and depends on `..`
        // (its parent, i.e. `a` itself).
        fixture_pkg(
            &dir.join("a"),
            "a",
            "1.0.0",
            &["./b"],
            "fn va()\n    return 1\nend\n",
        );
        fixture_pkg(
            &dir.join("a").join("b"),
            "b",
            "1.0.0",
            &[".."],
            "fn vb()\n    return 2\nend\n",
        );
        let err = install(
            &PackageSpec::Local {
                path: dir.join("a"),
            },
            &opts,
        )
        .unwrap_err();
        assert!(err.contains("dependency cycle: a -> b -> a"), "{err}");
        // a -> b (no cycle): both installed, lockfile has both.
        fixture_pkg(
            &dir.join("a").join("b"),
            "b",
            "1.0.0",
            &[],
            "fn vb()\n    return 2\nend\n",
        );
        let reports = install(
            &PackageSpec::Local {
                path: dir.join("a"),
            },
            &opts,
        )
        .unwrap();
        assert_eq!(reports.len(), 2);
        assert_eq!(reports[0].name, "a");
        assert_eq!(reports[1].name, "b");
        let lock = read_lockfile(&project.join("gg.lock")).unwrap();
        assert_eq!(lock.packages.len(), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn install_detects_version_conflicts() {
        let dir = temp_dir("conflict");
        let project = dir.join("proj");
        let global = dir.join("home").join(".gg").join("packages");
        std::fs::create_dir_all(&project).unwrap();
        let opts = install_opts(&project, &global);
        fixture_pkg(
            &dir.join("top").join("bar1"),
            "bar",
            "1.0.0",
            &[],
            "fn v()\n    return 1\nend\n",
        );
        fixture_pkg(
            &dir.join("top").join("bar2"),
            "bar",
            "2.0.0",
            &[],
            "fn v()\n    return 2\nend\n",
        );
        fixture_pkg(
            &dir.join("top"),
            "top",
            "1.0.0",
            &["./bar1", "./bar2"],
            "fn v()\n    return 0\nend\n",
        );
        let err = install(
            &PackageSpec::Local {
                path: dir.join("top"),
            },
            &opts,
        )
        .unwrap_err();
        assert!(err.contains("version conflict for package 'bar'"), "{err}");
        assert!(err.contains("1.0.0"), "{err}");
        assert!(err.contains("2.0.0"), "{err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn install_from_lockfile_reinstalls_pinned_versions() {
        let dir = temp_dir("lock-install");
        let project = dir.join("proj");
        let global = dir.join("home").join(".gg").join("packages");
        std::fs::create_dir_all(&project).unwrap();
        let opts = install_opts(&project, &global);
        fixture_pkg(
            &dir.join("src"),
            "ciao",
            "1.0.0",
            &[],
            "fn v()\n    return 1\nend\n",
        );
        install(
            &PackageSpec::Local {
                path: dir.join("src"),
            },
            &opts,
        )
        .unwrap();
        // Wipe the store: `gg install` with no arguments brings
        // it back from the lockfile.
        std::fs::remove_dir_all(&global).unwrap();
        assert!(!global.join("ciao").is_dir());
        let reports = install_from_lockfile(&opts).unwrap();
        assert_eq!(reports.len(), 1);
        assert_eq!(reports[0].name, "ciao");
        assert!(!reports[0].already_installed);
        assert!(global.join("ciao").join("1.0.0").join("lib.gg").is_file());
        // A second reinstall is a no-op (already installed).
        let reports = install_from_lockfile(&opts).unwrap();
        assert!(reports[0].already_installed);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn list_and_remove_cover_both_roots() {
        let dir = temp_dir("list-remove");
        let project = dir.join("proj");
        let global = dir.join("home").join(".gg").join("packages");
        let local = project.join("gg_packages");
        std::fs::create_dir_all(&project).unwrap();
        fixture_pkg(
            &dir.join("src"),
            "ciao",
            "1.0.0",
            &[],
            "fn v()\n    return 1\nend\n",
        );
        assert!(list_installed(&global, &local).is_empty());
        let opts = install_opts(&project, &global);
        install(
            &PackageSpec::Local {
                path: dir.join("src"),
            },
            &opts,
        )
        .unwrap();
        let mut local_opts = install_opts(&project, &global);
        local_opts.local = true;
        fixture_pkg(
            &dir.join("src2"),
            "altro",
            "0.2.0",
            &[],
            "fn v()\n    return 2\nend\n",
        );
        install(
            &PackageSpec::Local {
                path: dir.join("src2"),
            },
            &local_opts,
        )
        .unwrap();
        let listed = list_installed(&global, &local);
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0].name, "altro");
        assert_eq!(listed[0].location, Location::Local);
        assert_eq!(listed[1].name, "ciao");
        assert_eq!(listed[1].location, Location::Global);
        let removed = remove_package("ciao", &global, &local).unwrap();
        assert_eq!(removed.len(), 1);
        assert!(!global.join("ciao").exists());
        assert!(remove_package("ciao", &global, &local).is_err());
        assert!(remove_package("mai-esistito", &global, &local).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A local git repository with a package in it, for the
    /// clone tests (offline: `git clone` accepts a path).
    fn git_repo(dir: &Path, name: &str, version: &str) -> String {
        std::fs::create_dir_all(dir).unwrap();
        let run = |args: &[&str]| {
            let output = ProcCommand::new("git")
                .args(args)
                .current_dir(dir)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "git {} failed: {}",
                args.join(" "),
                String::from_utf8_lossy(&output.stderr)
            );
            String::from_utf8(output.stdout).unwrap()
        };
        run(&["init", "-q"]);
        run(&["config", "user.email", "test@gg.local"]);
        run(&["config", "user.name", "Test"]);
        fixture_pkg(dir, name, version, &[], "fn v()\n    return 1\nend\n");
        run(&["add", "."]);
        run(&["commit", "-qm", "pacchetto"]);
        run(&["tag", &format!("v{version}")]);
        run(&["rev-parse", "HEAD"]).trim().to_string()
    }

    #[test]
    fn clone_from_a_git_repository_default_branch_tag_and_commit() {
        let dir = temp_dir("git-clone");
        let repo = dir.join("repo");
        let sha = git_repo(&repo, "ciao", "1.0.0");
        let url = repo.display().to_string();
        // Default branch.
        let dest = dir.join("clone-default");
        clone_repo(&url, &None, &dest).unwrap();
        assert!(dest.join("gg.toml").is_file());
        assert!(dest.join("lib.gg").is_file());
        // A tag.
        let dest = dir.join("clone-tag");
        clone_repo(&url, &Some(GitRef::Tag("v1.0.0".to_string())), &dest).unwrap();
        assert!(dest.join("gg.toml").is_file());
        // A commit: the checkout lands on exactly that sha.
        let dest = dir.join("clone-commit");
        clone_repo(&url, &Some(GitRef::Commit(sha.clone())), &dest).unwrap();
        assert_eq!(git_head_sha(&dest).unwrap(), sha);
        // A missing tag is a clear error, not a panic.
        let dest = dir.join("clone-nope");
        let err = clone_repo(&url, &Some(GitRef::Tag("v9.9.9".to_string())), &dest).unwrap_err();
        assert!(err.contains("git clone"), "{err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn package_entry_uses_the_manifest_entry_file() {
        let dir = temp_dir("entry");
        let global = dir.join("home").join(".gg").join("packages");
        let pkg = global.join("ciao").join("1.0.0");
        std::fs::create_dir_all(&pkg).unwrap();
        std::fs::write(
            pkg.join("gg.toml"),
            "name = \"ciao\"\nversion = \"1.0.0\"\nentry = \"src/main.gg\"\n",
        )
        .unwrap();
        std::fs::create_dir_all(pkg.join("src")).unwrap();
        std::fs::write(pkg.join("src/main.gg"), "fn v()\n    return 1\nend\n").unwrap();
        let entry = resolve_package_entry("ciao", &global, None).unwrap();
        assert!(entry.ends_with("src/main.gg"), "{}", entry.display());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
