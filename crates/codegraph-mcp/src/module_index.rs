// ABOUTME: Maps an import's text to the indexed file it names, per the importer's language,
// ABOUTME: following code-review-graph's resolvers; anything else (Go, packages) stays text.

use crate::rust_crates::{CrateLayout, RustCrates};
use std::collections::HashMap;

/// Extensions code-review-graph appends to a relative TS/JS import, in order.
const JS_EXTENSIONS: [&str; 7] = [".ts", ".tsx", ".js", ".jsx", ".vue", ".mts", ".cts"];
/// ESM writes `./foo.js` for `foo.ts`: the only case where a suffix is replaced.
const JS_SUFFIX_SUBSTITUTIONS: [(&str, &[&str]); 4] = [
    (".js", &[".ts", ".tsx", ".jsx"]),
    (".jsx", &[".ts", ".tsx"]),
    (".mjs", &[".mts", ".ts", ".tsx"]),
    (".cjs", &[".cts", ".ts", ".tsx"]),
];

#[derive(Clone, Copy, PartialEq)]
enum ImportLanguage {
    Python,
    Script,
    Rust,
    Java,
}

fn language(importer: &str) -> Option<ImportLanguage> {
    let extension = importer.rsplit_once('.')?.1;
    Some(match extension {
        "py" | "pyi" => ImportLanguage::Python,
        "ts" | "tsx" | "js" | "jsx" | "mjs" | "cjs" | "mts" | "cts" | "vue" => {
            ImportLanguage::Script
        }
        "rs" => ImportLanguage::Rust,
        "java" => ImportLanguage::Java,
        _ => return None,
    })
}

/// A path split into segments, remembering whether it was absolute.
#[derive(Clone)]
struct PathParts<'a> {
    absolute: bool,
    segments: Vec<&'a str>,
}

impl<'a> PathParts<'a> {
    fn new(path: &'a str) -> Self {
        Self {
            absolute: path.starts_with('/'),
            segments: path
                .split('/')
                .filter(|s| !s.is_empty() && *s != ".")
                .collect(),
        }
    }

    fn parent(&self) -> Self {
        let mut parent = self.clone();
        parent.segments.pop();
        parent
    }

    fn join(&self, tail: &str) -> String {
        let mut joined = String::new();
        if self.absolute {
            joined.push('/');
        }
        for segment in &self.segments {
            joined.push_str(segment);
            joined.push('/');
        }
        joined.push_str(tail);
        joined
    }
}

/// Collapses `.`/`..` segments, keeping a leading `/`.
pub(crate) fn normalize(path: &str) -> String {
    let absolute = path.starts_with('/');
    let mut segments: Vec<&str> = Vec::new();
    for segment in path.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                segments.pop();
            }
            other => segments.push(other),
        }
    }
    let joined = segments.join("/");
    if absolute {
        format!("/{joined}")
    } else {
        joined
    }
}

/// The module path of `file` below `source_root`: its directories plus the file stem, except
/// that `lib.rs`/`main.rs`/`mod.rs` name their directory's module. Outside the root: none.
fn rust_current_module_parts<'a>(file: &'a str, source_root: &str) -> Vec<&'a str> {
    let relative = if source_root.is_empty() {
        Some(file)
    } else {
        file.strip_prefix(source_root)
            .and_then(|rest| rest.strip_prefix('/'))
    };
    let Some(relative) = relative else {
        return Vec::new();
    };
    let mut parts: Vec<&str> = relative.split('/').filter(|s| !s.is_empty()).collect();
    if let Some(name) = parts.pop()
        && !matches!(name, "lib.rs" | "main.rs" | "mod.rs")
    {
        parts.push(name.strip_suffix(".rs").unwrap_or(name));
    }
    parts
}

pub(crate) struct ModuleIndex {
    /// Normalized path → the indexed file path as given.
    files: HashMap<String, String>,
    /// Cargo layouts that anchor Rust paths; empty means every Rust import stays text.
    rust_crates: RustCrates,
}

impl ModuleIndex {
    pub(crate) fn new<'a>(files: impl IntoIterator<Item = &'a str>) -> Self {
        Self {
            files: files
                .into_iter()
                .map(|file| (normalize(file), file.to_string()))
                .collect(),
            rust_crates: RustCrates::default(),
        }
    }

    /// Supplies the Cargo layouts Rust imports resolve through.
    // The indexer wiring (Task 4) is the first non-test caller.
    #[allow(dead_code)]
    pub(crate) fn with_rust_crates(mut self, crates: RustCrates) -> Self {
        self.rust_crates = crates;
        self
    }

    fn file(&self, path: &str) -> Option<&str> {
        self.files.get(&normalize(path)).map(String::as_str)
    }

    /// The indexed file the import `text`, written in `importer`, names.
    pub(crate) fn resolve(&self, importer: &str, text: &str) -> Option<&str> {
        let text = text.trim().trim_matches(|c| c == '"' || c == '\'');
        if text.is_empty() {
            return None;
        }
        match language(importer)? {
            ImportLanguage::Python => self.python(importer, text),
            ImportLanguage::Script => self.script(importer, text),
            ImportLanguage::Rust => self.rust(importer, text),
            ImportLanguage::Java => self.java(importer, text),
        }
    }

    /// Relative modules climb one package per extra dot; absolute ones are looked up from
    /// the caller's directory upwards, package (`__init__.py`) before module.
    fn python(&self, importer: &str, text: &str) -> Option<&str> {
        let module = text.trim_start_matches('.');
        let level = text.len() - module.len();
        let relative = module.replace('.', "/");
        let candidates = |dir: &PathParts| -> Option<&str> {
            if relative.is_empty() {
                return self.file(&dir.join("__init__.py"));
            }
            self.file(&dir.join(&format!("{relative}/__init__.py")))
                .or_else(|| self.file(&dir.join(&format!("{relative}.py"))))
        };
        let mut dir = PathParts::new(importer).parent();
        if level > 0 {
            for _ in 1..level {
                dir.segments.pop()?;
            }
            return candidates(&dir);
        }
        // A package folder is not on `sys.path`: `pkg/typing.py` never answers `import typing`.
        let stdlib =
            crate::python_stdlib::is_stdlib_module(module.split('.').next().unwrap_or_default());
        loop {
            let hidden = stdlib && self.file(&dir.join("__init__.py")).is_some();
            if !hidden && let Some(file) = candidates(&dir) {
                return Some(file);
            }
            dir.segments.pop()?;
        }
    }

    /// Only relative specifiers name a file; bare package names stay text.
    fn script(&self, importer: &str, text: &str) -> Option<&str> {
        if !(text.starts_with("./") || text.starts_with("../")) {
            return None;
        }
        let base = PathParts::new(importer).parent().join(text);
        if let Some(file) = self.file(&base) {
            return Some(file);
        }
        // Append, never replace: `./outlet.entity` is `outlet.entity.ts`.
        for extension in JS_EXTENSIONS {
            if let Some(file) = self.file(&format!("{base}{extension}")) {
                return Some(file);
            }
        }
        for (suffix, replacements) in JS_SUFFIX_SUBSTITUTIONS {
            if let Some(stem) = base.strip_suffix(suffix) {
                for replacement in replacements {
                    if let Some(file) = self.file(&format!("{stem}{replacement}")) {
                        return Some(file);
                    }
                }
            }
        }
        JS_EXTENSIONS
            .iter()
            .find_map(|extension| self.file(&format!("{base}/index{extension}")))
    }

    /// The module file for `parts` below a crate's source root: `<parts>.rs`, then
    /// `<parts>/mod.rs`; no parts is the crate root file. Lookups are exact-case.
    fn rust_module_file(&self, layout: &CrateLayout, parts: &[&str]) -> Option<&str> {
        if parts.is_empty() {
            return self.file(&layout.root_file);
        }
        let path = parts.join("/");
        let under_root = |tail: String| -> String {
            if layout.source_root.is_empty() {
                tail
            } else {
                format!("{}/{tail}", layout.source_root)
            }
        };
        self.file(&under_root(format!("{path}.rs")))
            .or_else(|| self.file(&under_root(format!("{path}/mod.rs"))))
    }

    /// Follows code-review-graph's `_resolve_rust_module_file`. The importer's Cargo crate
    /// anchors `crate::` (crate root), `self::` (the importer's module), `super::` (parent
    /// modules) and path-dependency names (that crate's root). Any other first segment must be
    /// a module of the crate root. Only the last segment may be an item; without a crate, or
    /// when a path climbs above the root, the import stays text.
    fn rust(&self, importer: &str, text: &str) -> Option<&str> {
        let segments: Vec<&str> = text.split("::").filter(|s| !s.is_empty()).collect();
        let (&first, rest) = segments.split_first()?;
        let importer_path = normalize(importer);
        let own = self.rust_crates.crate_of(&importer_path)?;
        let mut layout = own;
        let mut current: Vec<&str> = Vec::new();
        let mut remaining: &[&str] = &segments;
        let anchored;
        let mut resolved = match first {
            "crate" => {
                anchored = true;
                remaining = rest;
                self.rust_module_file(layout, &[])?
            }
            "self" => {
                anchored = true;
                remaining = rest;
                current = rust_current_module_parts(&importer_path, &layout.source_root);
                self.file(&importer_path)?
            }
            "super" => {
                anchored = true;
                current = rust_current_module_parts(&importer_path, &layout.source_root);
                while let Some((&"super", tail)) = remaining.split_first() {
                    remaining = tail;
                    current.pop()?;
                }
                self.rust_module_file(layout, &current)?
            }
            _ => match self.rust_crates.dependency(own, first) {
                Some(dependency) => {
                    // Never follows the dependency's own dependencies: cycles terminate.
                    anchored = true;
                    remaining = rest;
                    layout = dependency;
                    self.rust_module_file(layout, &[])?
                }
                None => {
                    anchored = false;
                    self.rust_module_file(layout, &[])?
                }
            },
        };
        for (index, segment) in remaining.iter().enumerate() {
            current.push(*segment);
            if let Some(found) = self.rust_module_file(layout, &current) {
                resolved = found;
                continue;
            }
            // Only a module the path reached exports the final item: a lone `Vec` reached none.
            if index != remaining.len() - 1 || (index == 0 && !anchored) {
                return None;
            }
            break;
        }
        let resolved_path = normalize(resolved);
        let inside = |dir: &str| dir.is_empty() || resolved_path.starts_with(&format!("{dir}/"));
        (inside(&own.dir) || inside(&layout.dir)).then_some(resolved)
    }

    /// `com.example.Foo` is `com/example/Foo.java` under some ancestor of the importer;
    /// a static member import retries without its last segment.
    fn java(&self, importer: &str, text: &str) -> Option<&str> {
        if text.ends_with(".*") {
            return None;
        }
        let find = |module: &str| -> Option<&str> {
            let relative = format!("{}.java", module.replace('.', "/"));
            let mut dir = PathParts::new(importer).parent();
            loop {
                if let Some(file) = self.file(&dir.join(&relative)) {
                    return Some(file);
                }
                dir.segments.pop()?;
            }
        };
        find(text).or_else(|| find(text.rsplit_once('.')?.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rust_crates::RustCrates;
    use std::collections::{BTreeMap, BTreeSet};

    /// `files` indexed, with `manifests` (directory → `Cargo.toml` text) read as Cargo layouts.
    fn rust_index(files: &[&str], manifests: &[(&str, &str)]) -> ModuleIndex {
        let set: BTreeSet<String> = files.iter().map(|f| f.to_string()).collect();
        let manifests: BTreeMap<String, String> = manifests
            .iter()
            .map(|(dir, text)| (dir.to_string(), text.to_string()))
            .collect();
        ModuleIndex::new(files.iter().copied())
            .with_rust_crates(RustCrates::from_manifests(&manifests, &set))
    }

    const CRATE_FILES: [&str; 3] = ["crate/src/lib.rs", "crate/src/m.rs", "crate/src/m/inner.rs"];
    const CRATE_MANIFEST: (&str, &str) = ("crate", "[package]\nname = \"krate\"\n");

    fn index() -> ModuleIndex {
        let files = [
            "app/caller.py",
            "app/sample.py",
            "app/core.py",
            "pkg/__init__.py",
            "pkg/core.py",
            "tools/config.py",
            "web/src/x.ts",
            "web/src/util.ts",
            "web/src/outlet.ts",
            "web/src/outlet.entity.ts",
            "web/src/lib/index.ts",
            "web/react.ts",
            "go/errors.go",
            "go/main.go",
            "crate/src/lib.rs",
            "crate/src/m.rs",
            "crate/src/m/inner.rs",
            "j/src/com/example/Foo.java",
            "j/src/com/example/Main.java",
        ];
        rust_index(&files, &[CRATE_MANIFEST])
    }

    #[test]
    fn python_modules_resolve_from_the_callers_directory_upwards() {
        let index = index();
        assert_eq!(
            index.resolve("app/caller.py", "sample"),
            Some("app/sample.py")
        );
        assert_eq!(
            index.resolve("app/caller.py", "app.sample"),
            Some("app/sample.py")
        );
        assert_eq!(
            index.resolve("app/caller.py", ".sample"),
            Some("app/sample.py")
        );
        assert_eq!(
            index.resolve("app/caller.py", "pkg"),
            Some("pkg/__init__.py")
        );
        assert_eq!(
            index.resolve("app/caller.py", "pkg.core"),
            Some("pkg/core.py")
        );
        // `core` next to the caller wins over `pkg/core.py`.
        assert_eq!(index.resolve("app/caller.py", "core"), Some("app/core.py"));
        // `tools/config.py` is not on the caller's path.
        assert_eq!(index.resolve("app/caller.py", "config"), None);
        assert_eq!(
            index.resolve("app/caller.py", "tests.fixtures.sample"),
            None
        );
        assert_eq!(index.resolve("app/caller.py", "os"), None);
    }

    #[test]
    fn a_package_folder_never_answers_a_standard_library_import() {
        let index = ModuleIndex::new([
            "pkg/__init__.py",
            "pkg/typing.py",
            "pkg/b.py",
            "loose/typing.py",
        ]);
        assert_eq!(index.resolve("pkg/a.py", "typing"), None);
        assert_eq!(index.resolve("pkg/a.py", "typing.x"), None);
        assert_eq!(index.resolve("pkg/a.py", "b"), Some("pkg/b.py"));
        // Relative imports and folders without `__init__.py` still find it.
        assert_eq!(index.resolve("pkg/a.py", ".typing"), Some("pkg/typing.py"));
        assert_eq!(
            index.resolve("loose/a.py", "typing"),
            Some("loose/typing.py")
        );
    }

    #[test]
    fn script_imports_resolve_only_relative_specifiers() {
        let index = index();
        assert_eq!(
            index.resolve("web/src/x.ts", "./util"),
            Some("web/src/util.ts")
        );
        assert_eq!(
            index.resolve("web/src/x.ts", "./util.js"),
            Some("web/src/util.ts")
        );
        assert_eq!(
            index.resolve("web/src/x.ts", "./outlet.entity"),
            Some("web/src/outlet.entity.ts")
        );
        assert_eq!(
            index.resolve("web/src/x.ts", "./lib"),
            Some("web/src/lib/index.ts")
        );
        assert_eq!(
            index.resolve("web/src/x.ts", "../react"),
            Some("web/react.ts")
        );
        assert_eq!(index.resolve("web/src/x.ts", "react"), None);
        assert_eq!(index.resolve("web/x.ts", "./util"), None);
    }

    #[test]
    fn rust_paths_anchor_at_the_crate_root_or_the_current_module() {
        let index = index();
        let lib = "crate/src/lib.rs";
        assert_eq!(index.resolve(lib, "crate::m::f"), Some("crate/src/m.rs"));
        assert_eq!(index.resolve(lib, "crate::m"), Some("crate/src/m.rs"));
        assert_eq!(
            index.resolve(lib, "crate::m::inner::g"),
            Some("crate/src/m/inner.rs")
        );
        assert_eq!(index.resolve(lib, "crate::missing::deep::g"), None);
        assert_eq!(
            index.resolve("crate/src/m.rs", "self::inner::g"),
            Some("crate/src/m/inner.rs")
        );
        assert_eq!(
            index.resolve("crate/src/m/inner.rs", "super::f"),
            Some("crate/src/m.rs")
        );
        assert_eq!(index.resolve(lib, "serde::Serialize"), None);
        assert_eq!(index.resolve(lib, "std::fmt"), None);
    }

    #[test]
    fn rust_unanchored_first_segment_is_a_crate_root_module() {
        let index = index();
        let lib = "crate/src/lib.rs";
        assert_eq!(index.resolve(lib, "m::f"), Some("crate/src/m.rs"));
        assert_eq!(
            index.resolve("crate/src/m/inner.rs", "m::inner"),
            Some("crate/src/m/inner.rs")
        );
        assert_eq!(index.resolve(lib, "Vec::new"), None);
        assert_eq!(index.resolve(lib, "Vec"), None);
        assert_eq!(index.resolve(lib, "serde::Serialize"), None);
    }

    #[test]
    fn rust_path_dependency_resolves_to_its_lib_rs() {
        let files = [
            "app/src/main.rs",
            "other-lib/src/lib.rs",
            "other-lib/src/text.rs",
        ];
        let index = rust_index(
            &files,
            &[
                (
                    "app",
                    "[package]\nname = \"app\"\n[dependencies]\nother-lib = { path = \"../other-lib\" }\n",
                ),
                ("other-lib", "[package]\nname = \"other-lib\"\n"),
            ],
        );
        let main = "app/src/main.rs";
        assert_eq!(
            index.resolve(main, "other_lib::text::shout"),
            Some("other-lib/src/text.rs")
        );
        assert_eq!(
            index.resolve(main, "other-lib::text"),
            Some("other-lib/src/text.rs")
        );
        assert_eq!(
            index.resolve(main, "other_lib::X"),
            Some("other-lib/src/lib.rs")
        );
        assert_eq!(
            index.resolve(main, "other_lib"),
            Some("other-lib/src/lib.rs")
        );
        assert_eq!(index.resolve(main, "other_lib::missing::X"), None);
        // The dependency's modules are not the importer's: `text` alone is not in `app`.
        assert_eq!(index.resolve(main, "text::shout"), None);
    }

    #[test]
    fn rust_without_a_manifest_stays_text() {
        let index = rust_index(&CRATE_FILES, &[]);
        let lib = "crate/src/lib.rs";
        assert_eq!(index.resolve(lib, "crate::m"), None);
        assert_eq!(index.resolve(lib, "m::f"), None);
        assert_eq!(index.resolve("crate/src/m/inner.rs", "super::f"), None);
        assert_eq!(index.resolve("crate/src/m.rs", "self::inner"), None);
    }

    #[test]
    fn rust_super_above_the_root_and_self_in_root_files() {
        let index = index();
        let lib = "crate/src/lib.rs";
        assert_eq!(index.resolve(lib, "super::x"), None);
        assert_eq!(index.resolve("crate/src/m.rs", "super::super::x"), None);
        assert_eq!(index.resolve(lib, "self::m"), Some("crate/src/m.rs"));
        assert_eq!(
            index.resolve(lib, "self::m::inner::g"),
            Some("crate/src/m/inner.rs")
        );
        assert_eq!(index.resolve(lib, "self::Item"), Some("crate/src/lib.rs"));
        assert_eq!(
            index.resolve("crate/src/m.rs", "super::m"),
            Some("crate/src/m.rs")
        );
    }

    #[test]
    fn rust_module_files_match_case_exactly() {
        let upper = rust_index(
            &["crate/src/lib.rs", "crate/src/Glob.rs"],
            &[CRATE_MANIFEST],
        );
        assert_eq!(upper.resolve("crate/src/lib.rs", "crate::glob::x"), None);
        assert_eq!(upper.resolve("crate/src/lib.rs", "glob::x"), None);
        let lower = rust_index(
            &["crate/src/lib.rs", "crate/src/glob.rs"],
            &[CRATE_MANIFEST],
        );
        assert_eq!(lower.resolve("crate/src/lib.rs", "crate::Glob::x"), None);
        assert_eq!(
            lower.resolve("crate/src/lib.rs", "crate::glob::x"),
            Some("crate/src/glob.rs")
        );
    }

    #[test]
    fn java_classes_resolve_under_an_ancestor_and_other_languages_stay_text() {
        let index = index();
        let main = "j/src/com/example/Main.java";
        assert_eq!(
            index.resolve(main, "com.example.Foo"),
            Some("j/src/com/example/Foo.java")
        );
        assert_eq!(
            index.resolve(main, "com.example.Foo.member"),
            Some("j/src/com/example/Foo.java")
        );
        assert_eq!(index.resolve(main, "com.example.*"), None);
        assert_eq!(index.resolve("go/main.go", "errors"), None);
    }
}
