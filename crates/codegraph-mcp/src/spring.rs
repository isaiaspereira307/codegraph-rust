// ABOUTME: Spring DI call resolution over the whole project, ported from CRG spring_resolver.py.
// ABOUTME: Calls on injected fields go to the unique bean implementation or the injected type.

use codegraph_core::{CodeNode, EdgeRelationship, EdgeType, Language, NodeId, NodeType, Span};
use codegraph_parser::languages::java::{HANDLES, INJECTS, PUBLISHES};
use std::collections::{BTreeSet, HashMap};

fn qualified(node: &CodeNode) -> Option<&str> {
    node.metadata
        .attributes
        .get("qualified_name")
        .map(String::as_str)
}

fn attribute<'n>(node: &'n CodeNode, key: &str) -> Option<&'n str> {
    node.metadata.attributes.get(key).map(String::as_str)
}

fn is_class(node: &CodeNode) -> bool {
    matches!(
        node.node_type,
        Some(NodeType::Class | NodeType::Interface | NodeType::Struct)
    )
}

fn is_callable(node: &CodeNode) -> bool {
    match &node.node_type {
        Some(NodeType::Function) => true,
        Some(NodeType::Other(kind)) => kind == "Test",
        _ => false,
    }
}

fn is_test(node: &CodeNode) -> bool {
    attribute(node, "is_test") == Some("true")
        || matches!(&node.node_type, Some(NodeType::Other(kind)) if kind == "Test")
}

/// Rewrites the target of every Java CALLS edge whose receiver is a field injected by Spring
/// (an INJECTS edge with that `field_name` from the caller's class), as CRG's
/// `resolve_spring_di_calls` does after a build. The new target is the method on the injected
/// type's single non-test implementation (INHERITS) when the type is an interface, abstract or
/// unknown; otherwise on the one class with that Java identity; otherwise on the type as
/// written. Returns the number of edges rewritten.
pub(crate) fn resolve_spring_di_calls(nodes: &[CodeNode], edges: &mut [EdgeRelationship]) -> usize {
    let by_id: HashMap<NodeId, &CodeNode> = nodes.iter().map(|n| (n.id, n)).collect();
    let source = |id: &NodeId| by_id.get(id).copied();
    let is_java = |node: &CodeNode| node.language == Some(Language::Java);

    // (class qualified name, field name) -> (injected type as written, Java identity).
    let mut field_map: HashMap<(String, String), (String, Option<String>)> = HashMap::new();
    for edge in edges.iter() {
        if edge.edge_type != EdgeType::Other(INJECTS.into()) {
            continue;
        }
        let (Some(class), Some(field)) = (
            source(&edge.from).and_then(qualified),
            edge.metadata.get("field_name"),
        ) else {
            continue;
        };
        field_map.insert(
            (class.to_string(), field.clone()),
            (edge.to.clone(), edge.metadata.get("java_type").cloned()),
        );
    }
    if field_map.is_empty() {
        return 0;
    }

    let mut java_kinds: HashMap<String, BTreeSet<String>> = HashMap::new();
    let mut class_quals_by_type: HashMap<String, BTreeSet<String>> = HashMap::new();
    let mut types_by_simple_name: HashMap<String, BTreeSet<String>> = HashMap::new();
    for node in nodes.iter().filter(|n| is_java(n) && is_class(n)) {
        let Some(qual) = qualified(node) else {
            continue;
        };
        let bare = node.name.to_string();
        // The enclosing types belong to the identity: app.Outer.Sink is not app.Sink.
        let local_name = qual.split_once("::").map_or(qual, |(_, local)| local);
        let java_type = match attribute(node, "java_package") {
            Some(package) if !package.is_empty() => format!("{package}.{local_name}"),
            _ => bare.clone(),
        };
        class_quals_by_type
            .entry(java_type.clone())
            .or_default()
            .insert(qual.to_string());
        types_by_simple_name
            .entry(bare.clone())
            .or_default()
            .insert(java_type.clone());
        if let Some(kind) = attribute(node, "java_kind") {
            java_kinds
                .entry(java_type.clone())
                .or_default()
                .insert(kind.to_string());
            if java_type != bare {
                java_kinds.entry(bare).or_default().insert(kind.to_string());
            }
        }
    }

    // Methods by (class qualified name, name), and by (class simple name, name) keeping the
    // shortest qualified name.
    let mut method_to_qual: HashMap<(String, String), String> = HashMap::new();
    for node in nodes.iter().filter(|n| is_java(n) && is_callable(n)) {
        let (Some(qual), Some(parent)) = (qualified(node), attribute(node, "parent_name")) else {
            continue;
        };
        let file = qual.split_once("::").map_or(qual, |(file, _)| file);
        let name = node.name.to_string();
        method_to_qual.insert(
            (format!("{file}::{parent}"), name.clone()),
            qual.to_string(),
        );
        let bare_key = (
            parent.rsplit('.').next().unwrap_or(parent).to_string(),
            name,
        );
        match method_to_qual.get(&bare_key) {
            Some(previous) if previous.len() <= qual.len() => {}
            _ => {
                method_to_qual.insert(bare_key, qual.to_string());
            }
        }
    }

    let node_is_test: HashMap<&str, bool> = nodes
        .iter()
        .filter_map(|n| Some((qualified(n)?, is_test(n))))
        .collect();

    // Java identity (or base text) -> qualified names of the classes that extend/implement it.
    let mut implementors: HashMap<String, BTreeSet<String>> = HashMap::new();
    for edge in edges.iter() {
        if edge.edge_type != EdgeType::Extends || edge.metadata.contains_key("analyzer") {
            continue;
        }
        let Some(class) = source(&edge.from)
            .filter(|n| matches!(n.language, Some(Language::Java | Language::Kotlin)))
        else {
            continue;
        };
        let Some(class) = qualified(class) else {
            continue;
        };
        let parent = edge.metadata.get("java_type").unwrap_or(&edge.to).clone();
        implementors
            .entry(parent)
            .or_default()
            .insert(class.to_string());
    }

    let mut resolved = 0;
    for edge in edges.iter_mut() {
        if edge.edge_type != EdgeType::Calls || edge.metadata.contains_key("spring_resolved") {
            continue;
        }
        let Some(caller) = source(&edge.from).filter(|n| is_java(n)) else {
            continue;
        };
        let (Some(receiver), Some(source_qual)) =
            (edge.metadata.get("receiver"), qualified(caller))
        else {
            continue;
        };
        // A prior qualification may be wrong; the receiver says more.
        let method_name = match edge.to.split_once("::") {
            Some((_, after)) => after.rsplit('.').next().unwrap_or(after).to_string(),
            None => edge.to.clone(),
        };
        let Some((prefix, after)) = source_qual.split_once("::") else {
            continue;
        };
        let enclosing_class = match after.split_once('.') {
            Some((class, _)) => format!("{prefix}::{class}"),
            None => source_qual.to_string(),
        };
        let Some((injected_type, java_type)) = field_map.get(&(enclosing_class, receiver.clone()))
        else {
            continue;
        };

        let mut type_key = java_type.clone().unwrap_or_else(|| injected_type.clone());
        // A bare identity names a repository type only when one type has that simple name.
        if let Some(same_name) = types_by_simple_name.get(&type_key)
            && same_name.len() == 1
        {
            type_key = same_name.iter().next().unwrap().clone();
        }
        let source_is_test = node_is_test.get(source_qual).copied().unwrap_or(false);
        let kind_values = java_kinds
            .get(&type_key)
            .filter(|kinds| !kinds.is_empty())
            .or_else(|| {
                (type_key != *injected_type)
                    .then(|| java_kinds.get(injected_type))
                    .flatten()
            });
        let java_kind = kind_values
            .filter(|kinds| kinds.len() == 1)
            .and_then(|kinds| kinds.iter().next());
        let mut impls: Vec<&String> = match java_kind.map(String::as_str) {
            None | Some("interface" | "abstract") => {
                let mut all: BTreeSet<&String> = BTreeSet::new();
                for key in [&type_key, injected_type] {
                    all.extend(implementors.get(key).into_iter().flatten());
                }
                all.into_iter().collect()
            }
            _ => Vec::new(),
        };
        if !source_is_test {
            impls.retain(|i| !node_is_test.get(i.as_str()).copied().unwrap_or(false));
        }
        let new_target = if let [implementation] = impls.as_slice() {
            method_to_qual
                .get(&((*implementation).clone(), method_name.clone()))
                .cloned()
                .unwrap_or_else(|| format!("{implementation}.{method_name}"))
        } else {
            let type_bare = injected_type.rsplit('.').next().unwrap_or(injected_type);
            match class_quals_by_type.get(&type_key) {
                Some(declared) if declared.len() == 1 => {
                    let declared = declared.iter().next().unwrap();
                    method_to_qual
                        .get(&(declared.clone(), method_name.clone()))
                        .cloned()
                        .unwrap_or_else(|| format!("{declared}.{method_name}"))
                }
                _ => method_to_qual
                    .get(&(type_bare.to_string(), method_name.clone()))
                    .cloned()
                    .unwrap_or_else(|| format!("{injected_type}.{method_name}")),
            }
        };

        let java_type = java_type.clone();
        let injected_type = injected_type.clone();
        edge.to = new_target;
        edge.metadata
            .insert("spring_resolved".into(), "true".into());
        edge.metadata.insert("injected_type".into(), injected_type);
        if let Some(java_type) = java_type {
            edge.metadata.insert("java_type".into(), java_type);
        }
        resolved += 1;
    }
    resolved
}

/// Rebuilds publisher-to-listener CALLS from the project-wide `PUBLISHES`/`HANDLES` edges that
/// point at the same virtual `event::` node, as CRG's `event_resolver` does after a build.
/// The derived edges point at the listener's qualified name so the normal symbol resolution
/// binds them like any other call; they are stamped for idempotent reindexing.
pub(crate) fn resolve_spring_events(
    nodes: &[CodeNode],
    edges: &mut Vec<EdgeRelationship>,
) -> usize {
    let is_event =
        |kind: &EdgeType| matches!(kind, EdgeType::Other(k) if k == PUBLISHES || k == HANDLES);
    let identity = |edge: &EdgeRelationship| {
        let target = edge
            .metadata
            .get("event_type")
            .cloned()
            .filter(|s| !s.is_empty())?;
        Some(format!("event::{target}"))
    };

    let by_id: HashMap<NodeId, &CodeNode> = nodes.iter().map(|node| (node.id, node)).collect();
    let mut publishers: HashMap<String, Vec<(NodeId, String, u32, Option<Span>)>> = HashMap::new();
    let mut listeners: HashMap<String, Vec<NodeId>> = HashMap::new();
    for edge in edges.iter() {
        if !is_event(&edge.edge_type) || !edge.to.starts_with("event::") {
            continue;
        }
        let Some(target) = identity(edge) else {
            continue;
        };
        if edge.edge_type == EdgeType::Other(PUBLISHES.into()) {
            let Some(node) = by_id.get(&edge.from) else {
                continue;
            };
            let line = edge
                .metadata
                .get("line")
                .and_then(|line| line.parse().ok())
                .unwrap_or(node.location.line);
            publishers.entry(target).or_default().push((
                edge.from,
                node.location.file_path.to_string(),
                line,
                edge.span.clone(),
            ));
        } else if edge.metadata.get("resolution").map(String::as_str)
            == Some("spring_event_listener")
        {
            listeners.entry(target).or_default().push(edge.from);
        }
    }
    if publishers.is_empty() || listeners.is_empty() {
        return 0;
    }

    let mut emitted = 0;
    for (target, publishers) in publishers {
        let Some(listeners) = listeners.get(&target) else {
            continue;
        };
        let identity = target
            .strip_prefix("event::")
            .unwrap_or(&target)
            .to_string();
        for (from, file, line, span) in publishers {
            for listener in listeners {
                let Some(target_node) = by_id.get(listener) else {
                    continue;
                };
                let Some(target) = qualified(target_node) else {
                    continue;
                };
                if edges.iter().any(|edge| {
                    edge.edge_type == EdgeType::Calls
                        && edge.from == from
                        && edge.to == target
                        && edge.metadata.contains_key("spring_event_resolved")
                }) {
                    continue;
                }
                edges.push(EdgeRelationship {
                    from,
                    to: target.to_string(),
                    edge_type: EdgeType::Calls,
                    metadata: HashMap::from([
                        ("spring_event_resolved".to_string(), "true".to_string()),
                        ("event_type".to_string(), identity.clone()),
                        (
                            "resolution".to_string(),
                            "spring_application_event".to_string(),
                        ),
                        ("confidence".to_string(), "0.95".to_string()),
                        ("confidence_tier".to_string(), "INFERRED".to_string()),
                        ("source_file".to_string(), file.clone()),
                        ("line".to_string(), line.to_string()),
                    ]),
                    span: span.clone(),
                });
                emitted += 1;
            }
        }
    }
    emitted
}

/// Points a functional WebFlux HANDLES edge at the handler named by its method reference. The
/// parser records the reference's receiver and byte span; the matching CALLS edge is resolved by
/// the same Java type lookup, so its target is the handler's real `NodeId`.
pub(crate) fn resolve_webflux_handlers(
    edges: &mut [EdgeRelationship],
    targets: &[Option<NodeId>],
) -> usize {
    let span_key = |span: &Span| format!("{}:{}", span.start_byte, span.end_byte);
    let mut resolved = 0;
    for index in 0..edges.len() {
        let (receiver, handler, handler_span) = {
            let edge = &edges[index];
            if edge.edge_type != EdgeType::Other(HANDLES.into())
                || !edge.metadata.contains_key("webflux_handler")
            {
                continue;
            }
            let (Some(receiver), Some(handler), Some(handler_span)) = (
                edge.metadata.get("handler_receiver"),
                edge.metadata.get("handler"),
                edge.metadata.get("handler_span"),
            ) else {
                continue;
            };
            (receiver.clone(), handler.clone(), handler_span.clone())
        };
        let candidate = (0..edges.len()).find(|&other| {
            let edge = &edges[other];
            edge.edge_type == EdgeType::Calls
                && edge.metadata.get("receiver").map(String::as_str) == Some(receiver.as_str())
                && edge.to.rsplit('.').next() == Some(handler.as_str())
                && edge
                    .span
                    .as_ref()
                    .is_some_and(|span| span_key(span) == handler_span)
        });
        if let Some(Some(id)) = candidate.and_then(|other| targets.get(other)) {
            edges[index].from = *id;
            resolved += 1;
        }
    }
    resolved
}

#[cfg(test)]
mod tests {
    use super::*;
    use codegraph_core::Location;

    fn node(qualified_name: &str, kind: NodeType, attributes: &[(&str, &str)]) -> CodeNode {
        let (file, local) = qualified_name.split_once("::").unwrap();
        let name = local.rsplit('.').next().unwrap();
        let mut node = CodeNode::new(
            name.to_string(),
            Some(kind),
            Some(Language::Java),
            Location {
                file_path: file.to_string(),
                line: 1,
                column: 0,
                end_line: None,
                end_column: None,
            },
        );
        let map = &mut node.metadata.attributes;
        map.insert("qualified_name".into(), qualified_name.into());
        if let Some((parent, _)) = local.rsplit_once('.') {
            map.insert("parent_name".into(), parent.into());
        }
        for (key, value) in attributes {
            map.insert((*key).into(), (*value).into());
        }
        node
    }

    fn class(qualified_name: &str, kind: &str) -> CodeNode {
        node(
            qualified_name,
            NodeType::Class,
            &[("java_package", "app"), ("java_kind", kind)],
        )
    }

    fn edge(
        from: &CodeNode,
        to: &str,
        edge_type: EdgeType,
        metadata: &[(&str, &str)],
    ) -> EdgeRelationship {
        EdgeRelationship {
            from: from.id,
            to: to.into(),
            edge_type,
            metadata: metadata
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            span: None,
        }
    }

    fn injects(from: &CodeNode, to: &str, field: &str) -> EdgeRelationship {
        let java_type = format!("app.{to}");
        edge(
            from,
            to,
            EdgeType::Other(INJECTS.into()),
            &[("field_name", field), ("java_type", &java_type)],
        )
    }

    fn inherits(from: &CodeNode, to: &str) -> EdgeRelationship {
        let java_type = format!("app.{to}");
        edge(from, to, EdgeType::Extends, &[("java_type", &java_type)])
    }

    fn call(from: &CodeNode, to: &str, receiver: &str) -> EdgeRelationship {
        edge(from, to, EdgeType::Calls, &[("receiver", receiver)])
    }

    /// `Svc.run` calls `repo.save()` and `other.save()` on fields of the given types.
    fn resolve(mut nodes: Vec<CodeNode>, extra: Vec<EdgeRelationship>) -> Vec<String> {
        let service = class("A.java::Svc", "concrete");
        let run = node("A.java::Svc.run", NodeType::Function, &[]);
        let mut edges = vec![
            injects(&service, "Repo", "repo"),
            call(&run, "A.java::Repo.save", "repo"),
            call(&run, "save", "other"),
        ];
        edges.extend(extra);
        nodes.extend([service, run]);
        resolve_spring_di_calls(&nodes, &mut edges);
        edges
            .iter()
            .filter(|e| e.edge_type == EdgeType::Calls)
            .map(|e| e.to.clone())
            .collect()
    }

    #[test]
    fn a_single_implementation_answers_an_injected_interface() {
        let repo = class("A.java::Repo", "interface");
        let jpa = class("B.java::JpaRepo", "concrete");
        let save = node("B.java::JpaRepo.save", NodeType::Function, &[]);
        let extends = inherits(&jpa, "Repo");
        assert_eq!(
            resolve(vec![repo, jpa, save], vec![extends]),
            vec!["B.java::JpaRepo.save", "save"]
        );
    }

    #[test]
    fn several_implementations_fall_back_to_the_declared_type() {
        let repo = class("A.java::Repo", "interface");
        let one = class("B.java::One", "concrete");
        let two = class("C.java::Two", "concrete");
        let edges = vec![inherits(&one, "Repo"), inherits(&two, "Repo")];
        assert_eq!(
            resolve(vec![repo, one, two], edges),
            vec!["A.java::Repo.save", "save"]
        );
    }

    #[test]
    fn a_concrete_injected_class_answers_itself() {
        let repo = class("A.java::Repo", "concrete");
        let sub = class("B.java::Sub", "concrete");
        let extends = inherits(&sub, "Repo");
        assert_eq!(
            resolve(vec![repo, sub], vec![extends]),
            vec!["A.java::Repo.save", "save"]
        );
    }

    #[test]
    fn test_implementations_do_not_count_for_production_callers() {
        let repo = class("A.java::Repo", "interface");
        let jpa = class("B.java::JpaRepo", "concrete");
        let fake = node(
            "FakeTest.java::FakeRepo",
            NodeType::Class,
            &[
                ("java_package", "app"),
                ("java_kind", "concrete"),
                ("is_test", "true"),
            ],
        );
        let edges = vec![inherits(&jpa, "Repo"), inherits(&fake, "Repo")];
        assert_eq!(
            resolve(vec![repo, jpa, fake], edges),
            vec!["B.java::JpaRepo.save", "save"]
        );
    }

    #[test]
    fn an_unknown_type_keeps_its_written_name() {
        assert_eq!(resolve(vec![], vec![]), vec!["Repo.save", "save"]);
    }
}
