// ABOUTME: Maps an import's text to the indexed file it names, per the importer's language,
// ABOUTME: following code-review-graph's resolvers; anything else (Go, packages) stays text.

use crate::rust_crates::{CrateLayout, RustCrates};
use codegraph_parser::languages::python::{PythonModuleEvidence, python_module_evidence};
use serde_json::Value;
use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::sync::Arc;

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

/// A local npm package discovered from a `package.json` (workspace member or package root).
struct JsPackage {
    dir: String,
    manifest: Value,
}

pub(crate) struct ModuleIndex {
    /// Normalized path → the indexed file path as given.
    files: HashMap<String, String>,
    /// Local package name → its root directory and manifest, as code-review-graph's workspace
    /// and npm alias resolvers use them.
    js_packages: HashMap<String, JsPackage>,
    /// `npm:` dependency aliases → the package they name.
    npm_aliases: HashMap<String, String>,
    /// Cargo layouts that anchor Rust paths; empty means every Rust import stays text.
    rust_crates: RustCrates,
    /// Lazily read top-level Python shim/re-export evidence, keyed by the indexed file path.
    python_evidence: RefCell<HashMap<String, Arc<PythonModuleEvidence>>>,
    /// Files currently being followed through `sys.modules`, to stop shim cycles.
    shim_stack: RefCell<Vec<String>>,
}

/// Directories containing a readable `package.json`, found by walking up from indexed files.
fn npm_manifests(files: &HashMap<String, String>) -> BTreeMap<String, Value> {
    let mut manifests = BTreeMap::new();
    for file in files.values() {
        let Some(mut dir) = Path::new(file).parent() else {
            continue;
        };
        loop {
            let key = dir.to_string_lossy().into_owned();
            if !manifests.contains_key(&key) {
                let manifest = dir.join("package.json");
                let value = std::fs::read_to_string(manifest)
                    .ok()
                    .and_then(|text| serde_json::from_str(&text).ok())
                    .unwrap_or(Value::Null);
                manifests.insert(key, value);
            }
            let Some(parent) = dir.parent() else {
                break;
            };
            dir = parent;
        }
    }
    manifests
}

fn workspace_patterns(manifest: &Value) -> Vec<String> {
    let workspaces = manifest.get("workspaces");
    let list = match workspaces {
        Some(Value::Array(patterns)) => Some(patterns),
        Some(Value::Object(object)) => object.get("packages").and_then(Value::as_array),
        _ => None,
    };
    list.map(|patterns| {
        patterns
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect()
    })
    .unwrap_or_default()
}

/// Expands a workspace pattern; one trailing `*` matches the member directories beneath it.
fn workspace_directories(base: &str, pattern: &str) -> Vec<String> {
    let mut current = base.to_string();
    let mut parts = pattern.split('/').peekable();
    while let Some(part) = parts.next() {
        if part == "*" && parts.peek().is_none() {
            let mut found: Vec<_> = std::fs::read_dir(&current)
                .into_iter()
                .flatten()
                .flatten()
                .filter(|entry| entry.path().is_dir())
                .map(|entry| entry.path().to_string_lossy().into_owned())
                .collect();
            found.sort();
            return found;
        }
        if part == ".." {
            current = Path::new(&current)
                .parent()
                .map(|parent| parent.to_string_lossy().into_owned())
                .unwrap_or(current);
        } else if !part.is_empty() && part != "." {
            current = join_path(&current, part);
        }
    }
    vec![current]
}

fn join_path(dir: &str, tail: &str) -> String {
    let tail = tail.trim_start_matches("./");
    if dir.is_empty() {
        tail.to_string()
    } else {
        format!("{}/{tail}", dir.trim_end_matches('/'))
    }
}

/// The package entry code-review-graph uses: `exports` first, then source/module/main.
fn package_entry(manifest: &Value, subpath: Option<&str>) -> Option<String> {
    if let Some(exports) = manifest.get("exports") {
        let key = match subpath {
            Some(subpath) => format!("./{subpath}"),
            None => ".".to_string(),
        };
        let selected = exports
            .as_object()
            .and_then(|object| object.get(&key))
            .unwrap_or(exports);
        if let Some(entry) = export_target(selected) {
            return Some(entry);
        }
    }
    match subpath {
        Some(subpath) => Some(subpath.to_string()),
        None => ["source", "module", "main"]
            .iter()
            .find_map(|field| manifest.get(*field).and_then(Value::as_str))
            .map(str::to_string),
    }
}

fn export_target(value: &Value) -> Option<String> {
    if let Some(target) = value.as_str() {
        return Some(target.to_string());
    }
    let object = value.as_object()?;
    ["source", "import", "require", "default", "node"]
        .iter()
        .find_map(|condition| object.get(*condition).and_then(export_target))
}

impl ModuleIndex {
    pub(crate) fn new<'a>(files: impl IntoIterator<Item = &'a str>) -> Self {
        let files: HashMap<String, String> = files
            .into_iter()
            .map(|file| (normalize(file), file.to_string()))
            .collect();
        let manifests = npm_manifests(&files);
        let mut js_packages = HashMap::new();
        let mut npm_aliases = HashMap::new();
        for (dir, manifest) in &manifests {
            if let Some(name) = manifest.get("name").and_then(Value::as_str) {
                let package = JsPackage {
                    dir: dir.clone(),
                    manifest: manifest.clone(),
                };
                let slot = js_packages.entry(name.to_string()).or_insert(package);
                if dir.len() > slot.dir.len() {
                    *slot = JsPackage {
                        dir: dir.clone(),
                        manifest: manifest.clone(),
                    };
                }
            }
            for section in [
                "dependencies",
                "devDependencies",
                "peerDependencies",
                "optionalDependencies",
            ] {
                let Some(deps) = manifest.get(section).and_then(Value::as_object) else {
                    continue;
                };
                for (alias, spec) in deps {
                    let Some(spec) = spec.as_str().and_then(|spec| spec.strip_prefix("npm:"))
                    else {
                        continue;
                    };
                    // `npm:real@^1` and `npm:@scope/real@^1` both name a local package.
                    let name = if let Some(rest) = spec.strip_prefix('@') {
                        let end = rest.find('@').unwrap_or(rest.len());
                        format!("@{}", &rest[..end])
                    } else {
                        spec.split('@').next().unwrap_or(spec).to_string()
                    };
                    npm_aliases.entry(alias.clone()).or_insert(name);
                }
            }
            for pattern in workspace_patterns(manifest) {
                for member in workspace_directories(dir, &pattern) {
                    let Ok(text) = std::fs::read_to_string(join_path(&member, "package.json"))
                    else {
                        continue;
                    };
                    let Ok(value) = serde_json::from_str::<Value>(&text) else {
                        continue;
                    };
                    if let Some(name) = value.get("name").and_then(Value::as_str) {
                        let slot = js_packages.entry(name.to_string());
                        slot.or_insert(JsPackage {
                            dir: member,
                            manifest: value,
                        });
                    }
                }
            }
        }
        Self {
            files,
            js_packages,
            npm_aliases,
            rust_crates: RustCrates::default(),
            python_evidence: RefCell::new(HashMap::new()),
            shim_stack: RefCell::new(Vec::new()),
        }
    }

    /// Supplies the Cargo layouts Rust imports resolve through.
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

    /// Top-level `sys.modules`/re-export evidence of one indexed Python file, read once.
    fn python_evidence(&self, file: &str) -> Arc<PythonModuleEvidence> {
        if let Some(evidence) = self.python_evidence.borrow().get(file) {
            return evidence.clone();
        }
        let source = file
            .ends_with(".py")
            .then(|| std::fs::read(file).ok())
            .flatten()
            .and_then(|bytes| String::from_utf8(bytes).ok());
        let evidence = Arc::new(
            source
                .as_deref()
                .map(python_module_evidence)
                .unwrap_or_default(),
        );
        self.python_evidence
            .borrow_mut()
            .insert(file.to_string(), evidence.clone());
        evidence
    }

    /// A module that swaps itself in `sys.modules` hands importers the module it names.
    fn follow_shim<'a>(&'a self, file: &'a str) -> &'a str {
        if self.shim_stack.borrow().iter().any(|path| path == file) {
            return file;
        }
        self.shim_stack.borrow_mut().push(file.to_string());
        let resolved = self
            .python_evidence(file)
            .shim_target
            .clone()
            .and_then(|target| self.python(file, &target))
            .map(|real| self.follow_shim(real))
            .unwrap_or(file);
        self.shim_stack.borrow_mut().pop();
        resolved
    }

    /// The file that defines `symbol` even when `file` only re-exports it. Keeps `file` when
    /// the origin is outside the repository or the exports form a cycle.
    pub(crate) fn reexport_origin<'a>(&'a self, file: &'a str, symbol: &str) -> &'a str {
        self.reexport(file, symbol, &mut Vec::new())
    }

    fn reexport<'a>(&'a self, file: &'a str, symbol: &str, seen: &mut Vec<String>) -> &'a str {
        if seen.iter().any(|path| path == file) {
            return file;
        }
        seen.push(file.to_string());
        let Some((module, original)) = self.python_evidence(file).reexports.get(symbol).cloned()
        else {
            return file;
        };
        let Some(origin) = self.python(file, &module) else {
            return file;
        };
        if origin == file {
            return file;
        }
        self.reexport(origin, &original, seen)
    }

    /// Relative modules climb one package per extra dot; absolute ones are looked up from
    /// the caller's directory upwards, package (`__init__.py`) before module.
    fn python(&self, importer: &str, text: &str) -> Option<&str> {
        let module = text.trim_start_matches('.');
        let level = text.len() - module.len();
        let relative = module.replace('.', "/");
        let candidates = |dir: &PathParts| -> Option<&str> {
            let file = if relative.is_empty() {
                self.file(&dir.join("__init__.py"))?
            } else {
                self.file(&dir.join(&format!("{relative}/__init__.py")))
                    .or_else(|| self.file(&dir.join(&format!("{relative}.py"))))?
            };
            Some(self.follow_shim(file))
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

    /// Bare specifiers name a file only when a local workspace/npm package declares the name.
    fn script_package(&self, text: &str) -> Option<&str> {
        // A scoped package's `@scope/name` slash is not a subpath separator.
        let separator = if let Some(rest) = text.strip_prefix('@') {
            rest.find('/').and_then(|scope_end| {
                text[scope_end + 2..]
                    .find('/')
                    .map(|end| scope_end + 2 + end)
            })
        } else {
            text.find('/')
        };
        let (name, subpath) = match separator {
            Some(index) => (&text[..index], Some(&text[index + 1..])),
            None => (text, None),
        };
        let name = self
            .npm_aliases
            .get(name)
            .map(String::as_str)
            .unwrap_or(name);
        let package = self.js_packages.get(name)?;
        let entry = package_entry(&package.manifest, subpath)?;
        let base = join_path(&package.dir, &entry);
        if let Some(file) = self.file(&base) {
            return Some(file);
        }
        for extension in JS_EXTENSIONS {
            if let Some(file) = self.file(&format!("{base}{extension}")) {
                return Some(file);
            }
        }
        JS_EXTENSIONS
            .iter()
            .find_map(|extension| self.file(&format!("{base}/index{extension}")))
    }

    /// Relative specifiers resolve as files; bare ones only through local packages.
    fn script(&self, importer: &str, text: &str) -> Option<&str> {
        if !(text.starts_with("./") || text.starts_with("../")) {
            return self.script_package(text);
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
                    // Like code-review-graph, extra `super` levels clamp at the crate root:
                    // `super::config::load` from a root file still names `config::load`.
                    if !current.is_empty() {
                        current.pop();
                    }
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

    /// Writes a small repository to a temp dir and indexes its paths.
    fn py_repo(files: &[(&str, &str)]) -> (tempfile::TempDir, Vec<String>) {
        let dir = tempfile::tempdir().unwrap();
        let mut paths = Vec::new();
        for (relative, source) in files {
            let path = dir.path().join(relative);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, source).unwrap();
            paths.push(path.to_string_lossy().into_owned());
        }
        (dir, paths)
    }

    #[test]
    fn python_sys_modules_shim_resolves_to_the_real_module() {
        let (_dir, paths) = py_repo(&[
            (
                "app/services/engine.py",
                "def run_model(x):\n    return x\n",
            ),
            (
                "app/services/billing.py",
                "from app.services.engine import run_model\n\ndef charge(): pass\n",
            ),
            (
                "billing.py",
                "import importlib\nimport sys\n\n\
                 sys.modules[__name__] = importlib.import_module(\"app.services.billing\")\n",
            ),
            ("caller.py", "import billing\n"),
        ]);
        let index = ModuleIndex::new(paths.iter().map(String::as_str));
        let caller = &paths[3];
        assert_eq!(
            index.resolve(caller, "billing"),
            Some(paths[1].as_str()),
            "a shim hands the importer the module it swaps in"
        );
        assert_eq!(
            index.reexport_origin(&paths[1], "run_model"),
            paths[0].as_str(),
            "a name re-exported by the real module is defined by its origin"
        );
    }

    #[test]
    fn python_shim_of_a_shim_terminates_at_the_real_module() {
        let (_dir, paths) = py_repo(&[
            ("billing.py", "def charge(): pass\n"),
            (
                "alias.py",
                "import importlib\nimport sys\n\n\
                 sys.modules[__name__] = importlib.import_module(\"billing\")\n",
            ),
            (
                "legacy.py",
                "import importlib\nimport sys\n\n\
                 sys.modules[__name__] = importlib.import_module(\"alias\")\n",
            ),
            ("caller.py", "import legacy\n"),
        ]);
        let index = ModuleIndex::new(paths.iter().map(String::as_str));
        assert_eq!(index.resolve(&paths[3], "legacy"), Some(paths[0].as_str()));
    }

    #[test]
    fn python_shim_cycle_keeps_one_of_the_files() {
        let (_dir, paths) = py_repo(&[
            (
                "a.py",
                "import importlib\nimport sys\n\n\
                 sys.modules[__name__] = importlib.import_module(\"b\")\n",
            ),
            (
                "b.py",
                "import importlib\nimport sys\n\n\
                 sys.modules[__name__] = importlib.import_module(\"a\")\n",
            ),
            ("caller.py", "import a\n"),
        ]);
        let index = ModuleIndex::new(paths.iter().map(String::as_str));
        assert!(index.resolve(&paths[2], "a").is_some());
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
    fn workspace_package_specifiers_resolve_to_the_package_entry() {
        let (_dir, paths) = py_repo(&[
            (
                "package.json",
                r#"{"name":"root","workspaces":["packages/*"]}"#,
            ),
            (
                "packages/shared/package.json",
                r#"{"name":"@ws/shared","main":"index.js"}"#,
            ),
            (
                "packages/shared/index.js",
                "export function shared() { return 1; }\n",
            ),
            ("src/consumer.js", "const shared = require('@ws/shared');\n"),
        ]);
        let index = ModuleIndex::new(paths.iter().map(String::as_str));
        assert_eq!(
            index.resolve(&paths[3], "@ws/shared"),
            Some(paths[2].as_str())
        );
        assert_eq!(index.resolve(&paths[3], "unknown-package"), None);
    }

    #[test]
    fn npm_aliases_and_export_subpaths_resolve_to_local_packages() {
        let (_dir, paths) = py_repo(&[
            (
                "package.json",
                r#"{"name":"root","dependencies":{"alias":"npm:@ws/shared@^1"}}"#,
            ),
            (
                "packages/shared/package.json",
                r#"{"name":"@ws/shared","exports":{".":"./index.js","./extra":"./extra.js"}}"#,
            ),
            ("packages/shared/index.js", "export const a = 1;\n"),
            ("packages/shared/extra.js", "export const b = 2;\n"),
            (
                "src/consumer.js",
                "require('alias'); require('@ws/shared/extra');\n",
            ),
        ]);
        let index = ModuleIndex::new(paths.iter().map(String::as_str));
        assert_eq!(index.resolve(&paths[4], "alias"), Some(paths[2].as_str()));
        assert_eq!(
            index.resolve(&paths[4], "@ws/shared/extra"),
            Some(paths[3].as_str())
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
        // Like code-review-graph, `super` clamps at the crate root: the path then names an
        // item of the root file rather than leaving the call as text.
        assert_eq!(index.resolve(lib, "super::x"), Some(lib));
        assert_eq!(
            index.resolve("crate/src/m.rs", "super::super::x"),
            Some(lib)
        );
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
