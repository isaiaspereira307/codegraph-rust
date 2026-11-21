// ABOUTME: Test-file and test-function detection ported from code-review-graph's parser.py.
// ABOUTME: Directory rules read only the path relative to the repository root, never above it.

use regex::Regex;
use std::sync::LazyLock;

/// Path separator or start of the repository-relative path (CRG `_SEP`).
const SEP: &str = r"(?:^|[\\/])";

fn compile(patterns: &[&str]) -> Vec<Regex> {
    patterns
        .iter()
        .map(|p| Regex::new(p).expect("valid test-detection pattern"))
        .collect()
}

/// CRG `_TEST_PATTERNS`: names that mark a test function.
static TEST_PATTERNS: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    compile(&[
        r"^test_",
        r"^Test",
        r"_test$",
        r"\.test\.",
        r"\.spec\.",
        r"_spec$",
    ])
});

/// CRG `_ABSOLUTE_PATH_RE`: a leading slash, a Windows drive letter or a UNC share.
static ABSOLUTE_PATH_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(?:/|[A-Za-z]:/|//)").unwrap());

/// CRG `_TEST_DIR_PATTERNS`: test directories inside the repository.
static TEST_DIR_PATTERNS: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    let patterns = [
        r"tests[\\/]",
        r"__tests__[\\/]",
        r"e2e[_-]?tests?[\\/]",
        r"testthat[\\/]",
        r"specs?[\\/].*\.(?:rb|rake)$",
    ];
    patterns
        .iter()
        .map(|p| Regex::new(&format!("{SEP}{p}")).unwrap())
        .collect()
});

/// CRG `_SINGULAR_TEST_DIR_RE`.
static SINGULAR_TEST_DIR_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(&format!(r"{SEP}test[\\/]")).unwrap());

/// CRG `_TEST_NAME_PATTERNS` (with `_STORYBOOK_FILE_RE`), matched against the basename.
static TEST_NAME_PATTERNS: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    compile(&[
        r"^test_.*\.py$",
        r"_test\.py$",
        r"^conftest\.py$",
        r"\.(?:test|spec)\.[cm]?[jt]sx?$",
        r"\.stories\.[jt]sx?$",
        r"_test\.go$",
        r"_(?:test|spec)\.rb$",
        r"[a-z0-9]Tests?\.(?:java|kt|kts|scala|groovy)$",
        r"[a-z0-9]Tests?\.cs$",
        r"[a-z0-9]Test\.php$",
        r"_test\.dart$",
        r"^test[_-].*\.[rR]$",
        r"(?i)\.tests\.ps1$",
        r"[_.]test\.resi?$",
        r"^runtests\.jl$",
    ])
});

/// CRG `_TEST_RUNNER_NAMES`.
const TEST_RUNNER_NAMES: &[&str] = &[
    "describe",
    "it",
    "test",
    "beforeEach",
    "afterEach",
    "beforeAll",
    "afterAll",
    "suite",
];

/// CRG `_TEST_ANNOTATIONS`.
const TEST_ANNOTATIONS: &[&str] = &[
    "Test",
    "ParameterizedTest",
    "RepeatedTest",
    "TestFactory",
    "org.junit.Test",
    "org.junit.jupiter.api.Test",
    "test",
    "tokio::test",
    "async_std::test",
    "rstest",
    "rstest::rstest",
    "proptest",
];

/// CRG `is_test_file` for a path relative to the repository root. The shipped-package
/// exception for a singular `test/` directory (`_is_shipped_test_package`) needs the
/// filesystem and is not ported: such files always count as test code.
pub fn is_test_file(rel_path: &str) -> bool {
    let normalized = rel_path.replace('\\', "/");
    let basename = normalized.rsplit('/').next().unwrap_or(&normalized);
    if TEST_NAME_PATTERNS.iter().any(|p| p.is_match(basename)) {
        return true;
    }
    // An absolute path cannot be placed inside the repository without its root.
    if ABSOLUTE_PATH_RE.is_match(&normalized) {
        return false;
    }
    if TEST_DIR_PATTERNS.iter().any(|p| p.is_match(&normalized)) {
        return true;
    }
    SINGULAR_TEST_DIR_RE.is_match(&normalized)
}

/// Whether `name` is a JS/TS test runner call (`describe`, `it`, ...), CRG `_TEST_RUNNER_NAMES`.
pub fn is_test_runner_name(name: &str) -> bool {
    TEST_RUNNER_NAMES.contains(&name)
}

/// CRG `_is_test_function`: a test-like name, a test-runner name in a test file, or a test
/// annotation/decorator (given without its `@` or `#[ ]` wrapper).
pub fn is_test_function(name: &str, rel_path: &str, decorators: &[&str]) -> bool {
    TEST_PATTERNS.iter().any(|p| p.is_match(name))
        || (TEST_RUNNER_NAMES.contains(&name) && is_test_file(rel_path))
        || decorators.iter().any(|d| TEST_ANNOTATIONS.contains(d))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parent_dir_named_test_is_ignored() {
        assert!(!is_test_file("src/app.py")); // repo em /tmp/test/repo
        assert!(is_test_file("tests/app.py"));
        assert!(is_test_file("app_test.go"));
    }

    #[test]
    fn directory_rules_anchor_on_separators() {
        assert!(!is_test_file("src/latest/app.py"));
        assert!(!is_test_file("contests/app.py"));
        assert!(is_test_file("pkg/test/helpers.py"));
        assert!(is_test_file("src/__tests__/app.ts"));
        assert!(is_test_file("e2e-tests/flow.ts"));
        assert!(is_test_file("spec/models/user.rb"));
        assert!(!is_test_file("spec/openapi.yaml"));
        assert!(!is_test_file("/tmp/tests/app.py"));
    }

    #[test]
    fn file_name_rules() {
        for path in [
            "pkg/test_mod.py",
            "conftest.py",
            "web/app.spec.tsx",
            "ui/Button.stories.jsx",
            "src/FooTest.java",
            "src/FooTests.cs",
            "Unit.Tests.PS1",
            "/abs/x_test.go",
        ] {
            assert!(is_test_file(path), "{path}");
        }
        for path in ["src/ABTest.php", "src/APITest.java", "src/app.py"] {
            assert!(!is_test_file(path), "{path}");
        }
    }

    #[test]
    fn test_functions() {
        assert!(is_test_function("test_x", "src/a.py", &[]));
        assert!(is_test_function("TestParse", "a.go", &[]));
        assert!(is_test_function("parse_test", "a.rs", &[]));
        assert!(is_test_function("it", "tests/a.ts", &[]));
        assert!(!is_test_function("it", "src/a.ts", &[]));
        assert!(is_test_function("works", "src/a.rs", &["tokio::test"]));
        assert!(!is_test_function("works", "src/a.rs", &["derive(Debug)"]));
        assert!(!is_test_function("latest", "src/a.py", &[]));
    }
}
