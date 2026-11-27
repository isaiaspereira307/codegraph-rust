// ABOUTME: Finds flow entry points: uncalled functions, framework-decorated and conventional names.
// ABOUTME: Patterns are copied literally from code-review-graph flows.py (v2.4.0).

use crate::analysis::{AnalysisGraph, AnalysisNode};
use regex::Regex;
use std::collections::HashSet;
use std::sync::LazyLock;

fn compile(patterns: &[&str]) -> Vec<Regex> {
    patterns
        .iter()
        .map(|p| Regex::new(p).unwrap_or_else(|e| panic!("bad entry pattern {p}: {e}")))
        .collect()
}

static DECORATOR_PATTERNS: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    compile(&[
        r"(?i)app\.(get|post|put|delete|patch|route|websocket|on_event)",
        r"(?i)router\.(get|post|put|delete|patch|route)",
        r"(?i)blueprint\.(route|before_request|after_request)",
        r"(?i)(before|after)_(request|response)",
        r"(?i)click\.(command|group)",
        r"(?i)\w+\.(command|group)\b",
        r"(?i)(field|model)_(serializer|validator)",
        r"(?i)(celery\.)?(task|shared_task|periodic_task)",
        r"(?i)receiver",
        r"(?i)api_view",
        r"(?i)\baction\b",
        r"pytest\.(fixture|mark)",
        r"(?s)^(?:[\w.]+\.)?(?:Test|TestFactory|TestTemplate|ParameterizedTest|RepeatedTest|BeforeEach|AfterEach|BeforeAll|AfterAll|Before|After|BeforeClass|AfterClass|BeforeMethod|AfterMethod|BeforeSuite|AfterSuite|BeforeTest|AfterTest|BeforeGroups|AfterGroups|DataProvider)(?:\s*\(.*)?$",
        r"(?i)(override_settings|modify_settings)",
        r"(?i)(event\.)?listens_for",
        r"(?i)(Get|Post|Put|Delete|Patch|RequestMapping)Mapping",
        r"(?i)(Scheduled|EventListener|Bean|Configuration)",
        r"(?i)KafkaListener",
        r"(?i)(WorkflowMethod|ActivityMethod)",
        r"(?i)(Component|Injectable|Controller|Module|Guard|Pipe)",
        r"(?i)(Subscribe|Mutation|Query|Resolver)",
        r"(app|router)\.(get|post|put|delete|patch|use|all)\b",
        r"(?i)(HiltViewModel|AndroidEntryPoint|Inject)",
        r"(?i)\w+\.(tool|tool_plain|system_prompt|result_validator)\b",
        r"^tool\b",
        r"(?i)\w+\.(middleware|exception_handler|on_exception)\b",
        r"(?i)\w+\.route\b",
    ])
});

static NAME_PATTERNS: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    compile(&[
        r"^main$",
        r"^__main__$",
        r"^test_",
        r"^Test[A-Z]",
        r"^on_",
        r"^handle_",
        r"^handler$",
        r"^handle$",
        r"^lambda_handler$",
        r"^upgrade$",
        r"^downgrade$",
        r"^lifespan$",
        r"^get_db$",
        r"^on(Create|Start|Resume|Pause|Stop|Destroy|Bind|Receive)",
        r"^do(Get|Post|Put|Delete)$",
        r"^do_(GET|POST|PUT|DELETE|PATCH|HEAD|OPTIONS)$",
        r"^log_message$",
        r"^(middleware|errorHandler)$",
        r"^ng(OnInit|OnChanges|OnDestroy|DoCheck|AfterContentInit|AfterContentChecked|AfterViewInit|AfterViewChecked)$",
        r"^(transform|writeValue|registerOnChange|registerOnTouched|setDisabledState)$",
        r"^(canActivate|canDeactivate|canActivateChild|canLoad|canMatch|resolve)$",
        r"^(componentDidMount|componentDidUpdate|componentWillUnmount|shouldComponentUpdate|render)$",
    ])
});

static PHP_NAME_PATTERNS: LazyLock<Vec<Regex>> =
    LazyLock::new(|| compile(&[r"^(boot|register)$", r"^__invoke$"]));

fn has_framework_decorator(node: &AnalysisNode) -> bool {
    node.decorators
        .iter()
        .any(|d| DECORATOR_PATTERNS.iter().any(|p| p.is_match(d)))
}

fn matches_entry_name(node: &AnalysisNode) -> bool {
    if NAME_PATTERNS.iter().any(|p| p.is_match(&node.name)) {
        return true;
    }
    node.language.as_deref() == Some("php")
        && PHP_NAME_PATTERNS.iter().any(|p| p.is_match(&node.name))
}

/// Indices (in `g.nodes`, canonical order, no repeats) of the Function/Test nodes that start a flow.
pub fn detect_entry_points(g: &AnalysisGraph, include_tests: bool) -> Vec<usize> {
    let is_file = |qn: &str| g.index_of(qn).is_some_and(|i| g.nodes[i].kind == "File");
    let mut called: HashSet<&str> = HashSet::new();
    let mut called_outside_tests: HashSet<&str> = HashSet::new();
    for e in g.edges.iter().filter(|e| e.kind == "CALLS") {
        let source = g.index_of(&e.source).map(|i| &g.nodes[i]);
        if source.is_some_and(|n| n.kind == "File") || (source.is_none() && is_file(&e.source)) {
            continue;
        }
        called.insert(e.target.as_str());
        if source.is_none_or(|n| !n.is_test) {
            called_outside_tests.insert(e.target.as_str());
        }
    }
    let mut seen: HashSet<&str> = HashSet::new();
    let mut out = Vec::new();
    for i in g.canonical_nodes() {
        let node = &g.nodes[i];
        if node.kind != "Function" && node.kind != "Test" {
            continue;
        }
        if !include_tests && (node.is_test || node.kind == "Test") {
            continue;
        }
        let set = match node.language.as_deref() {
            Some("ruby" | "java") => &called_outside_tests,
            _ => &called,
        };
        let is_entry = !set.contains(node.qualified.as_str())
            || has_framework_decorator(node)
            || matches_entry_name(node);
        if is_entry && seen.insert(node.qualified.as_str()) {
            out.push(i);
        }
    }
    out
}
