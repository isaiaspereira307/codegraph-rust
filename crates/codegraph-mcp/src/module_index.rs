// ABOUTME: Maps import text (Python dotted modules, TS relative paths, Rust use paths) to the
// ABOUTME: one indexed file it names, by whole path segments; ambiguous modules map to nothing.

use std::collections::HashMap;

/// File stems that name their directory as a module (`pkg/__init__.py`, `m/mod.rs`, `x/index.ts`).
const DIRECTORY_STEMS: [&str; 3] = ["__init__", "mod", "index"];
/// Leading Rust path keywords that name no directory.
const RUST_ROOTS: [&str; 3] = ["crate", "self", "super"];

pub(crate) struct ModuleIndex {
    /// Every `/`-joined suffix of every file's extensionless path; `None` when ambiguous.
    modules: HashMap<String, Option<String>>,
}

fn segments(path: &str) -> Vec<&str> {
    path.split('/')
        .filter(|s| !s.is_empty() && *s != ".")
        .collect()
}

fn strip_extension(segment: &str) -> &str {
    match segment.rsplit_once('.') {
        Some((stem, _)) if !stem.is_empty() => stem,
        _ => segment,
    }
}

impl ModuleIndex {
    pub(crate) fn new<'a>(files: impl IntoIterator<Item = &'a str>) -> Self {
        let mut modules: HashMap<String, Option<String>> = HashMap::new();
        for file in files {
            let mut parts = segments(file);
            let Some(last) = parts.pop() else { continue };
            parts.push(strip_extension(last));
            let mut keys = vec![parts.clone()];
            if parts.len() > 1 && DIRECTORY_STEMS.contains(parts.last().unwrap()) {
                keys.push(parts[..parts.len() - 1].to_vec());
            }
            for key in keys {
                for start in 0..key.len() {
                    let module = key[start..].join("/");
                    let entry = modules
                        .entry(module)
                        .or_insert_with(|| Some(file.to_string()));
                    if entry.as_deref() != Some(file) {
                        *entry = None;
                    }
                }
            }
        }
        Self { modules }
    }

    fn lookup(&self, parts: &[&str]) -> Option<&str> {
        if parts.is_empty() {
            return None;
        }
        self.modules.get(&parts.join("/"))?.as_deref()
    }

    /// Like `lookup`, but `parts` must be the file's whole path (a resolved relative import).
    fn lookup_path(&self, parts: &[&str]) -> Option<&str> {
        let file = self.lookup(parts)?;
        let depth = segments(file).len();
        let directory_module = DIRECTORY_STEMS.iter().any(|stem| {
            segments(file)
                .last()
                .is_some_and(|last| strip_extension(last) == *stem)
        });
        (depth == parts.len() || (directory_module && depth == parts.len() + 1)).then_some(file)
    }

    /// The indexed file the import `text`, written in `importer`, names.
    pub(crate) fn resolve(&self, importer: &str, text: &str) -> Option<&str> {
        let text = text.trim().trim_matches(|c| c == '"' || c == '\'');
        let directory = || {
            let mut dir = segments(importer);
            dir.pop();
            dir
        };
        if text.starts_with("./") || text.starts_with("../") {
            // A path relative to the importing file (TS/JS).
            let mut parts = directory();
            for segment in segments(text) {
                if segment == ".." {
                    parts.pop()?;
                } else {
                    parts.push(segment);
                }
            }
            let last = parts.pop()?;
            parts.push(strip_extension(last));
            return self.lookup_path(&parts);
        }
        if text.starts_with('.') {
            // A Python relative module: each extra leading dot climbs one directory.
            let module = text.trim_start_matches('.');
            let mut parts = directory();
            for _ in 1..text.len() - module.len() {
                parts.pop()?;
            }
            parts.extend(module.split('.').filter(|s| !s.is_empty()));
            return self.lookup_path(&parts);
        }
        if text.contains("::") {
            // A Rust use path names an item inside its module; try the module itself, then
            // its parent.
            let mut parts: Vec<_> = text
                .split("::")
                .skip_while(|s| RUST_ROOTS.contains(s))
                .filter(|s| !s.is_empty() && !s.starts_with('{') && *s != "*")
                .collect();
            if let Some(file) = self.lookup(&parts) {
                return Some(file);
            }
            parts.pop();
            return self.lookup(&parts);
        }
        let text = text
            .strip_prefix("@/")
            .or_else(|| text.strip_prefix("~/"))
            .unwrap_or(text);
        let separator = if text.contains('/') { '/' } else { '.' };
        let parts: Vec<_> = text.split(separator).filter(|s| !s.is_empty()).collect();
        self.lookup(&parts)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modules_map_to_their_single_file() {
        let index = ModuleIndex::new([
            "app/caller.py",
            "app/sample.py",
            "pkg/__init__.py",
            "pkg/core.py",
            "other/core.py",
            "web/src/x.ts",
            "web/src/util.ts",
            "web/src/lib/index.ts",
            "src/m.rs",
            "src/lib.rs",
            "com/example/Foo.java",
        ]);
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
        // Two files answer to `core`; neither is evidence.
        assert_eq!(index.resolve("app/caller.py", "core"), None);
        assert_eq!(
            index.resolve("app/caller.py", "tests.fixtures.sample"),
            None
        );
        assert_eq!(index.resolve("app/caller.py", "os"), None);
        assert_eq!(
            index.resolve("web/src/x.ts", "./util"),
            Some("web/src/util.ts")
        );
        assert_eq!(
            index.resolve("web/src/x.ts", "./util.js"),
            Some("web/src/util.ts")
        );
        assert_eq!(
            index.resolve("web/src/x.ts", "./lib"),
            Some("web/src/lib/index.ts")
        );
        assert_eq!(
            index.resolve("web/src/x.ts", "../../src/m"),
            Some("src/m.rs")
        );
        // A relative path names one location, never a deeper file sharing its suffix.
        assert_eq!(
            index.resolve("web/x.ts", "./src/util"),
            Some("web/src/util.ts")
        );
        assert_eq!(index.resolve("x.ts", "./util"), None);
        assert_eq!(index.resolve("src/lib.rs", "crate::m::f"), Some("src/m.rs"));
        assert_eq!(index.resolve("src/lib.rs", "crate::m"), Some("src/m.rs"));
        assert_eq!(index.resolve("src/lib.rs", "std::fmt"), None);
        assert_eq!(
            index.resolve("x/Main.java", "com.example.Foo"),
            Some("com/example/Foo.java")
        );
    }
}
