// ABOUTME: Maps an import's text to the indexed file it names, per the importer's language,
// ABOUTME: following code-review-graph's resolvers; anything else (Go, packages) stays text.

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

pub(crate) struct ModuleIndex {
    /// Normalized path → the indexed file path as given.
    files: HashMap<String, String>,
}

impl ModuleIndex {
    pub(crate) fn new<'a>(files: impl IntoIterator<Item = &'a str>) -> Self {
        Self {
            files: files
                .into_iter()
                .map(|file| (normalize(file), file.to_string()))
                .collect(),
        }
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

    /// `crate::` paths start at the crate root (the nearest ancestor directory holding
    /// `lib.rs` or `main.rs`); `self::`/`super::` at the importer's module. Other first
    /// segments (std, extern crates) stay text. The last segment may be an item.
    fn rust(&self, importer: &str, text: &str) -> Option<&str> {
        let mut remaining: Vec<&str> = text.split("::").filter(|s| !s.is_empty()).collect();
        let first = *remaining.first()?;
        if !matches!(first, "crate" | "self" | "super") {
            return None;
        }
        let file = PathParts::new(importer);
        let mut root = file.parent();
        let root_file = loop {
            if let Some(found) = ["lib.rs", "main.rs"]
                .iter()
                .find_map(|name| self.file(&root.join(name)))
            {
                break found;
            }
            root.segments.pop()?;
        };
        let module_file = |parts: &[&str]| -> Option<&str> {
            if parts.is_empty() {
                return Some(root_file);
            }
            let path = parts.join("/");
            self.file(&root.join(&format!("{path}.rs")))
                .or_else(|| self.file(&root.join(&format!("{path}/mod.rs"))))
        };
        let mut current: Vec<&str> = Vec::new();
        if first == "crate" {
            remaining.remove(0);
        } else {
            // The importer's own module path below the crate root.
            current = file.segments[root.segments.len()..].to_vec();
            let stem = current.pop()?.trim_end_matches(".rs");
            if !matches!(stem, "lib" | "main" | "mod") {
                current.push(stem);
            }
            if first == "self" {
                remaining.remove(0);
            }
            while remaining.first() == Some(&"super") {
                remaining.remove(0);
                current.pop()?;
            }
        }
        let mut resolved = module_file(&current)?;
        for (index, segment) in remaining.iter().enumerate() {
            current.push(segment);
            match module_file(&current) {
                Some(found) => resolved = found,
                None if index == remaining.len() - 1 => break,
                None => return None,
            }
        }
        Some(resolved)
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

    fn index() -> ModuleIndex {
        ModuleIndex::new([
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
            "crate/src/serde.rs",
            "j/src/com/example/Foo.java",
            "j/src/com/example/Main.java",
        ])
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
