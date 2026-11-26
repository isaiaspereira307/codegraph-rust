// ABOUTME: Maps an import's text to the indexed file it names, per the importer's language,
// ABOUTME: following code-review-graph's resolvers; anything else (Go, packages) stays text.

use crate::rust_crates::{CrateLayout, RustCrates};
use codegraph_parser::languages::python::{PythonModuleEvidence, python_module_evidence};
use serde_json::Value;
use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
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
    Ruby,
    Php,
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
        "rb" => ImportLanguage::Ruby,
        "php" => ImportLanguage::Php,
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

/// Whether `path` is `dir` itself or one of its descendants.
fn is_under(path: &str, dir: &str) -> bool {
    dir.is_empty() || path == dir || path.starts_with(&format!("{}/", dir.trim_end_matches('/')))
}

/// The suffix of a Go import below a module path, if it is that module or one of its packages.
fn go_suffix<'a>(text: &'a str, module: &str) -> Option<&'a str> {
    if text == module {
        Some("")
    } else {
        text.strip_prefix(&format!("{module}/"))
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

/// A `go.mod` above an indexed file: its module path and local `replace` targets.
struct GoModule {
    dir: String,
    path: String,
    /// `old module path` → `new module path` (local targets are absolute directories).
    replaces: Vec<(String, String)>,
}

/// Reads the nearest `go.mod` of every ancestor directory, like CRG's `_go_module_chain`.
/// `root` turns the indexer's project-relative file names into real manifest paths; the
/// stored directories stay in the same spelling as `files`.
fn go_modules(files: &HashMap<String, String>, root: Option<&Path>) -> Vec<GoModule> {
    let mut found = BTreeMap::new();
    for file in files.values() {
        let path = Path::new(file);
        let mut dir = if path.is_absolute() || root.is_none() {
            path.to_path_buf()
        } else {
            root.unwrap().join(path)
        };
        loop {
            let key = dir.to_string_lossy().into_owned();
            let stored = |path: &Path| -> String {
                root.and_then(|root| path.strip_prefix(root).ok())
                    .map(|relative| relative.to_string_lossy().into_owned())
                    .unwrap_or_else(|| path.to_string_lossy().into_owned())
            };
            if !found.contains_key(&key) {
                let manifest = dir.join("go.mod");
                let value = std::fs::read_to_string(manifest).ok().and_then(|text| {
                    let mut path = None;
                    let mut replaces = Vec::new();
                    let mut in_block = false;
                    for line in text.lines() {
                        let line = line.split("//").next().unwrap_or("").trim();
                        if line.is_empty() {
                            continue;
                        }
                        if in_block {
                            if line == ")" {
                                in_block = false;
                            } else if let Some((old, new)) = replace_directive(line) {
                                let new = local_go_target(&dir, new);
                                replaces.push((old.to_string(), new));
                            }
                            continue;
                        }
                        if line == "replace (" {
                            in_block = true;
                        } else if let Some(value) = line.strip_prefix("module ") {
                            path = Some(value.trim().trim_matches('"').to_string());
                        } else if let Some(rest) = line.strip_prefix("replace ") {
                            if let Some((old, new)) = replace_directive(rest) {
                                let new = local_go_target(&dir, new);
                                replaces.push((old.to_string(), new));
                            }
                        }
                    }
                    path.map(|path| GoModule {
                        dir: stored(&dir),
                        path,
                        replaces,
                    })
                });
                found.insert(key, value);
            }
            let Some(parent) = dir.parent() else {
                break;
            };
            dir = parent.to_path_buf();
        }
    }
    found.into_values().flatten().collect()
}

/// A relative `replace` target is a directory next to the `go.mod` that declares it.
fn local_go_target(dir: &Path, target: &str) -> String {
    if target.starts_with('.') || target.starts_with('/') {
        normalize(&join_path(&dir.to_string_lossy(), target))
    } else {
        target.to_string()
    }
}

/// `old [version] => new [version]` from a single `replace` line.
fn replace_directive(line: &str) -> Option<(&str, &str)> {
    let (old, new) = line.split_once("=>")?;
    let old = old.split_whitespace().next()?;
    let new = new.split_whitespace().next()?;
    Some((old, new))
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
    /// Go modules above the indexed files; their imports name directories, not files.
    go_modules: Vec<GoModule>,
    /// Project root used to find `go.mod` for project-relative file names.
    go_root: Option<PathBuf>,
    /// Lazily read top-level Python shim/re-export evidence, keyed by the indexed file path.
    python_evidence: RefCell<HashMap<String, Arc<PythonModuleEvidence>>>,
    /// Files currently being followed through `sys.modules`, to stop shim cycles.
    shim_stack: RefCell<Vec<String>>,
    /// PSR-4 maps of each directory's `composer.json` (`None`: the directory has none).
    php_composer: RefCell<HashMap<String, Option<Psr4Mappings>>>,
}

/// `(namespace prefix, destination directories)`, longest prefix first.
type Psr4Mappings = Arc<Vec<(String, Vec<String>)>>;

/// Reads the PSR-4 maps of `<dir>composer.json`; `None` when the file is missing, and an
/// empty map when it is unreadable or has none (the nearest file still decides).
fn read_psr4(dir: &str) -> Option<Psr4Mappings> {
    let text = std::fs::read_to_string(format!("{dir}composer.json")).ok()?;
    let Ok(value) = serde_json::from_str::<Value>(&text) else {
        return Some(Arc::new(Vec::new()));
    };
    let mut combined: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for section in ["autoload", "autoload-dev"] {
        let Some(psr4) = value
            .get(section)
            .and_then(|section| section.get("psr-4"))
            .and_then(Value::as_object)
        else {
            continue;
        };
        for (raw_prefix, raw_paths) in psr4 {
            let prefix = raw_prefix.trim_matches('\\').to_string();
            let paths: Vec<&str> = match raw_paths {
                Value::String(path) => vec![path.as_str()],
                Value::Array(paths) => paths.iter().filter_map(Value::as_str).collect(),
                _ => Vec::new(),
            };
            let destinations = combined.entry(prefix).or_default();
            for path in paths {
                let destination = normalize(&join_path(dir.trim_end_matches('/'), path));
                if !destinations.contains(&destination) {
                    destinations.push(destination);
                }
            }
        }
    }
    let mut mappings: Vec<_> = combined
        .into_iter()
        .filter(|(_, destinations)| !destinations.is_empty())
        .collect();
    mappings.sort_by(|a, b| b.0.len().cmp(&a.0.len()).then_with(|| a.0.cmp(&b.0)));
    Some(Arc::new(mappings))
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
        let go_modules = go_modules(&files, None);
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
            go_modules,
            go_root: None,
            python_evidence: RefCell::new(HashMap::new()),
            shim_stack: RefCell::new(Vec::new()),
            php_composer: RefCell::new(HashMap::new()),
        }
    }

    /// Supplies the Cargo layouts Rust imports resolve through.
    pub(crate) fn with_rust_crates(mut self, crates: RustCrates) -> Self {
        self.rust_crates = crates;
        self
    }

    /// Supplies the project root so Go manifests can be found from relative file names.
    pub(crate) fn set_go_root(&mut self, root: &Path) {
        self.go_root = Some(root.to_path_buf());
        self.go_modules = go_modules(&self.files, Some(root));
    }

    fn file(&self, path: &str) -> Option<&str> {
        self.files.get(&normalize(path)).map(String::as_str)
    }

    /// The package directory a Go import names, when it holds indexed source. Unlike other
    /// languages, a Go import resolves to a directory; the edge keeps that directory as text.
    pub(crate) fn go_package(&self, importer: &str, text: &str) -> Option<String> {
        let importer = normalize(importer);
        let importer_dir = Path::new(&importer)
            .parent()?
            .to_string_lossy()
            .into_owned();
        let first = text.split('/').next().unwrap_or(text);
        if !first.contains('.') {
            return None;
        }
        let mut modules: Vec<&GoModule> = self
            .go_modules
            .iter()
            .filter(|module| is_under(&importer_dir, &module.dir))
            .collect();
        modules.sort_by(|a, b| b.dir.len().cmp(&a.dir.len()));
        for module in modules {
            if let Some(suffix) = go_suffix(text, &module.path) {
                let dir = if suffix.is_empty() {
                    module.dir.clone()
                } else {
                    join_path(&module.dir, suffix)
                };
                if self.go_holds_source(&dir) {
                    return Some(self.go_absolute(&dir));
                }
            }
            for (old, new) in &module.replaces {
                if !new.starts_with('/') {
                    continue;
                }
                if let Some(suffix) = go_suffix(text, old) {
                    let dir = if suffix.is_empty() {
                        new.clone()
                    } else {
                        join_path(new, suffix)
                    };
                    if self.go_holds_source(&dir) {
                        return Some(self.go_absolute(&dir));
                    }
                }
            }
        }
        for ancestor in Path::new(&importer_dir).ancestors() {
            let dir = join_path(&ancestor.to_string_lossy(), &format!("vendor/{text}"));
            if self.go_holds_source(&dir) {
                return Some(self.go_absolute(&dir));
            }
        }
        None
    }

    /// Emits directory targets in the same absolute spelling as indexed node paths.
    fn go_absolute(&self, dir: &str) -> String {
        match &self.go_root {
            Some(root) if !Path::new(dir).is_absolute() => {
                normalize(&join_path(&root.to_string_lossy(), dir))
            }
            _ => normalize(dir),
        }
    }

    /// Whether an indexed `.go` file sits directly in `dir`, as CRG's package source check.
    fn go_holds_source(&self, dir: &str) -> bool {
        let dir = self
            .go_root
            .as_ref()
            .and_then(|root| Path::new(dir).strip_prefix(root).ok())
            .map(|relative| relative.to_string_lossy().into_owned())
            .unwrap_or_else(|| dir.to_string());
        let prefix = format!("{}/", normalize(&dir).trim_end_matches('/'));
        self.files.values().any(|file| {
            let file = normalize(file);
            file.starts_with(&prefix)
                && file.ends_with(".go")
                && !file[prefix.len()..].contains('/')
        })
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
            ImportLanguage::Ruby => self.ruby(text),
            ImportLanguage::Php => self.php(importer, text),
        }
    }

    /// CRG's PHP module resolution: a literal include path relative to the importer; otherwise
    /// a class name through the nearest `composer.json` PSR-4 map, then as a namespace path
    /// (`A\B` -> `A/B.php`) from the importer's directory upward.
    fn php(&self, importer: &str, text: &str) -> Option<&str> {
        let importer_dir = PathParts::new(importer).parent();
        if text.contains('/') || text.ends_with(".php") {
            let target = if text.starts_with('/') {
                text.to_string()
            } else {
                importer_dir.join(text)
            };
            return self.file(&target);
        }
        let module = text.trim_start_matches('\\');
        if module.is_empty() {
            return None;
        }
        if let Some(file) = self.php_composer(&importer_dir, module) {
            return Some(file);
        }
        let relative = format!("{}.php", module.replace('\\', "/"));
        let mut dir = importer_dir;
        loop {
            if let Some(file) = self.file(&dir.join(&relative)) {
                return Some(file);
            }
            dir.segments.pop()?;
        }
    }

    /// The PSR-4 maps of the `composer.json` nearest above `dir` (`autoload` and
    /// `autoload-dev`), longest prefix first, like CRG's `_read_php_composer_psr4`.
    fn php_composer(&self, dir: &PathParts, module: &str) -> Option<&str> {
        let mut dir = dir.clone();
        let mappings = loop {
            let key = dir.join("");
            let cached = self.php_composer.borrow().get(&key).cloned();
            let mappings = match cached {
                Some(mappings) => mappings,
                None => {
                    let mappings = read_psr4(&key);
                    self.php_composer.borrow_mut().insert(key, mappings.clone());
                    mappings
                }
            };
            if let Some(mappings) = mappings {
                break mappings;
            }
            dir.segments.pop()?;
        };
        for (prefix, destinations) in mappings.iter() {
            let relative = if prefix.is_empty() {
                module
            } else if let Some(rest) = module
                .strip_prefix(prefix.as_str())
                .and_then(|rest| rest.strip_prefix('\\'))
            {
                rest
            } else {
                continue;
            };
            if relative.is_empty() {
                continue;
            }
            let relative = format!("{}.php", relative.replace('\\', "/"));
            for destination in destinations {
                if let Some(file) = self.file(&join_path(destination, &relative)) {
                    return Some(file);
                }
            }
        }
        None
    }

    /// Ruby `require_relative` targets are resolved by the extractor (relative to the
    /// importer, with `.rb` kept); find the indexed file that exact path names.
    fn ruby(&self, text: &str) -> Option<&str> {
        text.ends_with(".rb").then(|| self.file(text)).flatten()
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

    /// CRG `_java_same_package_file`: `Type.java` beside the importer, or the same package in a
    /// sibling source set that a test compiles against. A main source set sees no sibling.
    pub(crate) fn java_same_package_file(&self, importer: &str, class_name: &str) -> Option<&str> {
        if class_name.is_empty()
            || !class_name
                .chars()
                .all(|c| c.is_alphanumeric() || c == '_' || c == '$')
        {
            return None;
        }
        let file_name = format!("{class_name}.java");
        let parts = PathParts::new(importer);
        let own = self.file(&parts.parent().join(&file_name));
        if own.is_some() {
            return own;
        }
        let segments = &parts.segments;
        let java = segments.iter().position(|s| *s == "java")?;
        if java < 2 || segments[java - 2] != "src" {
            return None;
        }
        let source_set = segments[java - 1];
        if source_set == "main" {
            return None;
        }
        let package = &segments[java + 1..segments.len().saturating_sub(1)];
        for other in ["main", "test"] {
            if other == source_set {
                continue;
            }
            let mut candidate = PathParts {
                absolute: parts.absolute,
                segments: segments[..java - 2].to_vec(),
            };
            candidate
                .segments
                .extend(["src", other, "java"].iter().copied());
            candidate.segments.extend_from_slice(package);
            let found = self.file(&candidate.join(&file_name));
            if found.is_some() {
                return found;
            }
        }
        None
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
    fn php_imports_resolve_include_paths_composer_psr4_and_namespace_paths() {
        let (_dir, paths) = py_repo(&[
            (
                "composer.json",
                r#"{"autoload":{"psr-4":{"App\\":"src/","App\\Admin\\":["admin/"]}}}"#,
            ),
            ("src/Models/User.php", "<?php\n"),
            ("admin/Panel.php", "<?php\n"),
            ("Lib/Tool.php", "<?php\n"),
            ("src/Http/Controller.php", "<?php\n"),
        ]);
        let index = ModuleIndex::new(paths.iter().map(String::as_str));
        let importer = paths[4].as_str();
        assert_eq!(
            index.resolve(importer, "App\\Models\\User"),
            Some(paths[1].as_str())
        );
        // The longest PSR-4 prefix wins.
        assert_eq!(
            index.resolve(importer, "App\\Admin\\Panel"),
            Some(paths[2].as_str())
        );
        // Outside the PSR-4 map, the namespace path is searched upward from the importer.
        assert_eq!(
            index.resolve(importer, "Lib\\Tool"),
            Some(paths[3].as_str())
        );
        assert_eq!(
            index.resolve(importer, "../Models/User.php"),
            Some(paths[1].as_str())
        );
        assert_eq!(index.resolve(importer, "App\\Missing"), None);
        assert_eq!(index.resolve(importer, "Exception"), None);
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
    fn go_packages_resolve_to_directories_through_module_replace_and_vendor() {
        let (dir, paths) = py_repo(&[
            (
                "go.mod",
                "module example.com/probe\n\ngo 1.21\n\nreplace example.com/dep => ./third_party/dep\n",
            ),
            ("app/main.go", "package app\n"),
            ("pkg/helper/helper.go", "package helper\n"),
            ("third_party/dep/go.mod", "module example.com/dep\n"),
            ("third_party/dep/dep.go", "package dep\n"),
            (
                "vendor/example.com/vendored/vendored.go",
                "package vendored\n",
            ),
        ]);
        let index = ModuleIndex::new(paths.iter().map(String::as_str));
        let main = &paths[1];
        let root = dir.path().to_string_lossy();
        assert_eq!(
            index.go_package(main, "example.com/probe/pkg/helper"),
            Some(format!("{root}/pkg/helper"))
        );
        assert_eq!(
            index.go_package(main, "example.com/dep"),
            Some(format!("{root}/third_party/dep"))
        );
        assert_eq!(
            index.go_package(main, "example.com/vendored"),
            Some(format!("{root}/vendor/example.com/vendored"))
        );
        assert_eq!(index.go_package(main, "errors"), None);
        assert_eq!(index.go_package(main, "fmt"), None);
    }

    #[test]
    fn relative_go_files_resolve_through_the_project_root() {
        let (dir, _paths) = py_repo(&[
            (
                "go.mod",
                "module example.com/probe\n\nreplace example.com/dep => ./third_party/dep\n",
            ),
            ("app/main.go", "package app\n"),
            ("pkg/helper/helper.go", "package helper\n"),
            ("third_party/dep/dep.go", "package dep\n"),
        ]);
        let mut index = ModuleIndex::new([
            "app/main.go",
            "pkg/helper/helper.go",
            "third_party/dep/dep.go",
        ]);
        index.set_go_root(dir.path());
        let root = dir.path().to_string_lossy();
        assert_eq!(
            index.go_package("app/main.go", "example.com/probe/pkg/helper"),
            Some(format!("{root}/pkg/helper"))
        );
        assert_eq!(
            index.go_package("app/main.go", "example.com/dep"),
            Some(format!("{root}/third_party/dep"))
        );
    }

    #[test]
    fn java_same_package_and_qualified_types_find_their_files() {
        let index = index();
        let main = "j/src/com/example/Main.java";
        assert_eq!(
            index.java_same_package_file(main, "Foo"),
            Some("j/src/com/example/Foo.java")
        );
        assert_eq!(
            index.resolve(main, "com.example.Foo"),
            Some("j/src/com/example/Foo.java")
        );
        assert_eq!(index.java_same_package_file(main, "Missing"), None);
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
