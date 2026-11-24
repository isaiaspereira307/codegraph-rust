// ABOUTME: Reads Cargo.toml manifests into crate layouts and local path dependencies, following
// ABOUTME: code-review-graph's Rust project context; feeds cross-file module resolution.

// Consumed by the module resolver (Task 3) and the indexer wiring (Task 4).
#![allow(dead_code)]

use crate::module_index::normalize;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

const DEPENDENCY_SECTIONS: [&str; 3] = ["dependencies", "dev-dependencies", "build-dependencies"];

/// A local crate: where its sources live and which sibling crates it depends on by path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CrateLayout {
    pub dir: String,
    pub source_root: String,
    pub root_file: String,
    /// Alias (both `-` and `_` spellings) to the `dir` of another crate in the map.
    pub dependencies: BTreeMap<String, String>,
}

#[derive(Debug, Default)]
pub(crate) struct RustCrates {
    crates: BTreeMap<String, CrateLayout>,
}

/// Joins `rel` onto `base` and collapses it; `None` when it climbs above the root of `base`.
fn join_inside(base: &str, rel: &str) -> Option<String> {
    let joined = if base.is_empty() {
        rel.to_string()
    } else {
        format!("{}/{}", base.trim_end_matches('/'), rel)
    };
    let mut depth = 0usize;
    for segment in joined.split('/') {
        match segment {
            "" | "." => {}
            ".." => depth = depth.checked_sub(1)?,
            _ => depth += 1,
        }
    }
    Some(normalize(&joined))
}

fn parent_dir(path: &str) -> Option<&str> {
    path.rsplit_once('/').map(|(parent, _)| parent)
}

/// `dir` and every ancestor, ending with `""` for relative paths.
fn ancestors(dir: &str) -> Vec<String> {
    let mut out = vec![dir.to_string()];
    let mut current = dir;
    while let Some(parent) = parent_dir(current) {
        out.push(parent.to_string());
        current = parent;
    }
    if !dir.is_empty() && !dir.starts_with('/') {
        out.push(String::new());
    }
    out.dedup();
    out
}

fn dependency_specs(manifest: &toml::Table) -> BTreeMap<String, toml::Value> {
    let mut specs = BTreeMap::new();
    let mut collect = |table: &toml::Table| {
        for section in DEPENDENCY_SECTIONS {
            if let Some(deps) = table.get(section).and_then(toml::Value::as_table) {
                for (name, spec) in deps {
                    specs.insert(name.clone(), spec.clone());
                }
            }
        }
    };
    collect(manifest);
    if let Some(targets) = manifest.get("target").and_then(toml::Value::as_table) {
        for target in targets.values().filter_map(toml::Value::as_table) {
            collect(target);
        }
    }
    specs
}

fn layout_of(dir: &str, manifest: &toml::Table, files: &BTreeSet<String>) -> Option<CrateLayout> {
    manifest.get("package").and_then(toml::Value::as_table)?;
    let explicit = manifest
        .get("lib")
        .and_then(toml::Value::as_table)
        .and_then(|lib| lib.get("path"))
        .and_then(toml::Value::as_str)
        .and_then(|path| join_inside(dir, path))
        .filter(|root| files.contains(root));
    let (source_root, root_file) = match explicit {
        Some(root) => (parent_dir(&root).unwrap_or("").to_string(), root),
        None => {
            let src = join_inside(dir, "src")?;
            let root = ["lib.rs", "main.rs"]
                .iter()
                .filter_map(|name| join_inside(&src, name))
                .find(|candidate| files.contains(candidate))?;
            (src, root)
        }
    };
    Some(CrateLayout {
        dir: dir.to_string(),
        source_root,
        root_file,
        dependencies: BTreeMap::new(),
    })
}

impl RustCrates {
    /// `manifests` maps a manifest's directory to its `Cargo.toml` text; `files` holds the
    /// indexed paths, used to decide whether `[lib] path`, `src/lib.rs` or `src/main.rs` exist.
    pub(crate) fn from_manifests(
        manifests: &BTreeMap<String, String>,
        files: &BTreeSet<String>,
    ) -> Self {
        let parsed: BTreeMap<String, toml::Table> = manifests
            .iter()
            .filter_map(|(dir, text)| {
                let table = toml::from_str::<toml::Table>(text).ok()?;
                Some((normalize(dir), table))
            })
            .collect();
        let mut crates: BTreeMap<String, CrateLayout> = parsed
            .iter()
            .filter_map(|(dir, table)| {
                layout_of(dir, table, files).map(|layout| (dir.clone(), layout))
            })
            .collect();
        let known: BTreeSet<String> = crates.keys().cloned().collect();
        for (dir, table) in &parsed {
            if !known.contains(dir) {
                continue;
            }
            let workspace = ancestors(dir).into_iter().find_map(|ancestor| {
                let specs = parsed
                    .get(&ancestor)?
                    .get("workspace")?
                    .as_table()?
                    .get("dependencies")?
                    .as_table()?;
                Some((ancestor, specs))
            });
            let mut dependencies = BTreeMap::new();
            for (alias, raw) in dependency_specs(table) {
                let Some(mut spec) = raw.as_table() else {
                    continue;
                };
                let mut base = dir.as_str();
                if spec.get("workspace").and_then(toml::Value::as_bool) == Some(true) {
                    let Some((workspace_dir, specs)) = &workspace else {
                        continue;
                    };
                    let Some(inherited) = specs.get(&alias).and_then(toml::Value::as_table) else {
                        continue;
                    };
                    spec = inherited;
                    base = workspace_dir;
                }
                let Some(target) = spec
                    .get("path")
                    .and_then(toml::Value::as_str)
                    .and_then(|path| join_inside(base, path))
                else {
                    continue;
                };
                if !known.contains(&target) {
                    continue;
                }
                dependencies.insert(alias.replace('-', "_"), target.clone());
                dependencies.insert(alias, target);
            }
            if let Some(layout) = crates.get_mut(dir) {
                layout.dependencies = dependencies;
            }
        }
        Self { crates }
    }

    /// Reads every `Cargo.toml` above the indexed `.rs` files (up to `project_root`).
    pub(crate) fn discover(project_root: &Path, rust_files: &BTreeSet<String>) -> Self {
        let root_text = project_root.to_string_lossy().replace('\\', "/");
        let root_text = root_text.trim_end_matches('/');
        let mut seen: BTreeSet<String> = BTreeSet::new();
        let mut manifests = BTreeMap::new();
        for file in rust_files {
            let Some(start) = parent_dir(file) else {
                // A bare file name lives in the project root, whose key is "".
                Self::read_manifest(project_root, "", &mut seen, &mut manifests);
                continue;
            };
            for dir in ancestors(start) {
                if file.starts_with('/') && !dir.starts_with(root_text) {
                    break;
                }
                Self::read_manifest(project_root, &dir, &mut seen, &mut manifests);
                if dir == root_text {
                    break;
                }
            }
        }
        Self::from_manifests(&manifests, rust_files)
    }

    fn read_manifest(
        project_root: &Path,
        dir: &str,
        seen: &mut BTreeSet<String>,
        manifests: &mut BTreeMap<String, String>,
    ) {
        if !seen.insert(dir.to_string()) {
            return;
        }
        let path = if dir.starts_with('/') {
            Path::new(dir).join("Cargo.toml")
        } else {
            project_root.join(dir).join("Cargo.toml")
        };
        if let Ok(text) = std::fs::read_to_string(path) {
            manifests.insert(dir.to_string(), text);
        }
    }

    /// The layout of the deepest crate whose directory contains `file`.
    pub(crate) fn crate_of(&self, file: &str) -> Option<&CrateLayout> {
        let start = parent_dir(file).unwrap_or("");
        ancestors(start).iter().find_map(|dir| self.crates.get(dir))
    }

    pub(crate) fn dependency(&self, krate: &CrateLayout, alias: &str) -> Option<&CrateLayout> {
        self.crates.get(krate.dependencies.get(alias)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(paths: &[&str]) -> BTreeSet<String> {
        paths.iter().map(|p| p.to_string()).collect()
    }

    fn manifests(items: &[(&str, &str)]) -> BTreeMap<String, String> {
        items
            .iter()
            .map(|(d, t)| (d.to_string(), t.to_string()))
            .collect()
    }

    const PKG: &str = "[package]\nname = \"x\"\n";

    #[test]
    fn layout_prefers_lib_path_then_lib_rs_then_main_rs() {
        let m = manifests(&[
            (
                "a",
                "[package]\nname=\"a\"\n[lib]\npath = \"core/root.rs\"\n",
            ),
            ("b", PKG),
            ("c", PKG),
        ]);
        let files = set(&[
            "a/core/root.rs",
            "a/src/lib.rs",
            "b/src/lib.rs",
            "b/src/main.rs",
            "c/src/main.rs",
        ]);
        let crates = RustCrates::from_manifests(&m, &files);
        let a = crates.crate_of("a/core/x.rs").unwrap();
        assert_eq!(
            (a.root_file.as_str(), a.source_root.as_str()),
            ("a/core/root.rs", "a/core")
        );
        assert_eq!(
            crates.crate_of("b/src/x.rs").unwrap().root_file,
            "b/src/lib.rs"
        );
        assert_eq!(
            crates.crate_of("c/src/x.rs").unwrap().root_file,
            "c/src/main.rs"
        );
    }

    #[test]
    fn path_dependencies_register_both_spellings() {
        let m = manifests(&[
            (
                "app",
                "[package]\nname=\"app\"\n[dependencies]\nmy-crate = { path = \"../my-crate\" }\n",
            ),
            ("my-crate", PKG),
        ]);
        let files = set(&["app/src/lib.rs", "my-crate/src/lib.rs"]);
        let crates = RustCrates::from_manifests(&m, &files);
        let app = crates.crate_of("app/src/lib.rs").unwrap();
        assert_eq!(crates.dependency(app, "my-crate").unwrap().dir, "my-crate");
        assert_eq!(crates.dependency(app, "my_crate").unwrap().dir, "my-crate");
    }

    #[test]
    fn target_specific_and_dev_dependencies_count() {
        let m = manifests(&[
            (
                "app",
                "[package]\nname=\"app\"\n[target.'cfg(unix)'.dependencies]\nu = { path = \"../u\" }\n[dev-dependencies]\nd = { path = \"../d\" }\n",
            ),
            ("u", PKG),
            ("d", PKG),
        ]);
        let files = set(&["app/src/lib.rs", "u/src/lib.rs", "d/src/lib.rs"]);
        let crates = RustCrates::from_manifests(&m, &files);
        let app = crates.crate_of("app/src/lib.rs").unwrap();
        assert!(crates.dependency(app, "u").is_some());
        assert!(crates.dependency(app, "d").is_some());
    }

    #[test]
    fn workspace_dependencies_are_inherited() {
        let m = manifests(&[
            (
                "",
                "[workspace]\nmembers=[\"crates/*\"]\n[workspace.dependencies]\ndep = { path = \"crates/dep\" }\n",
            ),
            (
                "crates/app",
                "[package]\nname=\"app\"\n[dependencies]\ndep = { workspace = true }\n",
            ),
            ("crates/dep", PKG),
        ]);
        let files = set(&["crates/app/src/lib.rs", "crates/dep/src/lib.rs"]);
        let crates = RustCrates::from_manifests(&m, &files);
        let app = crates.crate_of("crates/app/src/lib.rs").unwrap();
        assert_eq!(crates.dependency(app, "dep").unwrap().dir, "crates/dep");
    }

    #[test]
    fn broken_manifests_and_outside_paths_are_skipped() {
        let m = manifests(&[
            ("bad", "this is = = not toml"),
            (
                "ok",
                "[package]\nname=\"ok\"\n[dependencies]\nout = { path = \"../../outside\" }\nnolayout = { path = \"../empty\" }\ngood = { path = \"../good\" }\n",
            ),
            ("good", PKG),
            ("empty", PKG),
        ]);
        let files = set(&["bad/src/lib.rs", "ok/src/lib.rs", "good/src/lib.rs"]);
        let crates = RustCrates::from_manifests(&m, &files);
        assert!(crates.crate_of("bad/src/lib.rs").is_none());
        let ok = crates.crate_of("ok/src/lib.rs").unwrap();
        assert_eq!(
            ok.dependencies.keys().cloned().collect::<Vec<_>>(),
            vec!["good"]
        );
        assert!(crates.crate_of("good/src/lib.rs").is_some());
    }

    #[test]
    fn crate_of_picks_the_deepest_package() {
        let m = manifests(&[("outer", PKG), ("outer/inner", PKG)]);
        let files = set(&["outer/src/lib.rs", "outer/inner/src/lib.rs"]);
        let crates = RustCrates::from_manifests(&m, &files);
        assert_eq!(
            crates.crate_of("outer/inner/src/a.rs").unwrap().dir,
            "outer/inner"
        );
        assert_eq!(crates.crate_of("outer/src/a.rs").unwrap().dir, "outer");
        assert!(crates.crate_of("elsewhere/a.rs").is_none());
    }

    #[test]
    fn dependency_cycles_do_not_recurse() {
        let m = manifests(&[
            (
                "a",
                "[package]\nname=\"a\"\n[dependencies]\nb = { path = \"../b\" }\n",
            ),
            (
                "b",
                "[package]\nname=\"b\"\n[dependencies]\na = { path = \"../a\" }\n",
            ),
        ]);
        let files = set(&["a/src/lib.rs", "b/src/lib.rs"]);
        let crates = RustCrates::from_manifests(&m, &files);
        let a = crates.crate_of("a/src/lib.rs").unwrap();
        let b = crates.dependency(a, "b").unwrap();
        assert_eq!(b.dir, "b");
        assert_eq!(crates.dependency(b, "a").unwrap().dir, "a");
        assert!(crates.dependency(a, "a").is_none());
    }

    #[test]
    fn discover_reads_manifests_from_disk() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        std::fs::create_dir_all(root.join("app/src")).unwrap();
        std::fs::write(root.join("app/Cargo.toml"), PKG).unwrap();
        std::fs::write(root.join("app/src/lib.rs"), "").unwrap();
        let files = set(&["app/src/lib.rs"]);
        let crates = RustCrates::discover(root, &files);
        assert_eq!(crates.crate_of("app/src/lib.rs").unwrap().dir, "app");
    }
}
