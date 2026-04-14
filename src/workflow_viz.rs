use std::collections::{BTreeMap, BTreeSet, HashMap};

use crate::config::{NodeKind, WorkflowFile, WorkflowNode};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WorkflowVizFormat {
    Mermaid,
    Ascii,
}

pub fn render(workflow: &WorkflowFile, format: WorkflowVizFormat) -> String {
    match format {
        WorkflowVizFormat::Mermaid => render_mermaid(workflow),
        WorkflowVizFormat::Ascii => render_ascii(workflow),
    }
}

fn render_mermaid(workflow: &WorkflowFile) -> String {
    let data = build_graph_data(workflow);
    let mut name_gen = MermaidNameGenerator::default();
    let mut node_names = BTreeMap::new();
    for node_id in &data.node_ids {
        node_names.insert(node_id.clone(), name_gen.next(node_id));
    }

    let mut lines = vec!["flowchart TD".to_string()];

    for node_id in &data.node_ids {
        let node_name = &node_names[node_id];
        let label = node_label(node_id, data.node_definitions.get(node_id), data.join_indegree.get(node_id).copied());
        lines.push(format!("  {node_name}[\"{}\"]", escape_mermaid_label(&label)));
    }

    for (from, to) in &data.success_edges {
        lines.push(format!(
            "  {} --> {}",
            node_names[from],
            node_names[to]
        ));
    }

    for (from, failure_to) in &data.failure_edges {
        lines.push(format!(
            "  {} -. \"failure\" .-> {}",
            node_names[from],
            node_names[failure_to]
        ));
    }

    lines.join("\n")
}

fn render_ascii(workflow: &WorkflowFile) -> String {
    let data = build_graph_data(workflow);
    let mut lines = vec!["Nodes".to_string(), "-----".to_string()];
    for node_id in &data.node_ids {
        let label = node_label(node_id, data.node_definitions.get(node_id), data.join_indegree.get(node_id).copied());
        lines.push(format!("- {label}"));
    }

    lines.push(String::new());
    lines.push("Success edges".to_string());
    lines.push("-------------".to_string());
    for (from, to) in &data.success_edges {
        lines.push(format!("- {from} -> {to}"));
    }

    lines.push(String::new());
    lines.push("Failure edges".to_string());
    lines.push("-------------".to_string());
    for (from, to) in &data.failure_edges {
        lines.push(format!("- {from} -x-> {to}"));
    }

    lines.join("\n")
}

#[derive(Default)]
struct MermaidNameGenerator {
    used: HashMap<String, usize>,
}

impl MermaidNameGenerator {
    fn next(&mut self, node_id: &str) -> String {
        let mut sanitized: String = node_id
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
            .collect();
        if sanitized.is_empty()
            || sanitized.as_bytes()[0].is_ascii_digit()
            || is_mermaid_reserved_keyword(&sanitized)
        {
            sanitized.insert(0, 'n');
            sanitized.insert(1, '_');
        }
        let counter = self.used.entry(sanitized.clone()).or_insert(0);
        *counter += 1;
        if *counter == 1 {
            sanitized
        } else {
            format!("{sanitized}_{}", *counter)
        }
    }
}

fn is_mermaid_reserved_keyword(name: &str) -> bool {
    // Mermaid parser keywords that should not be used as node IDs.
    matches!(name, "end" | "subgraph" | "graph" | "flowchart")
}

fn escape_mermaid_label(label: &str) -> String {
    label.replace('\\', "\\\\").replace('"', "\\\"")
}

struct GraphData<'a> {
    node_ids: BTreeSet<String>,
    node_definitions: HashMap<String, &'a WorkflowNode>,
    success_edges: Vec<(String, String)>,
    failure_edges: Vec<(String, String)>,
    join_indegree: HashMap<String, usize>,
}

fn build_graph_data(workflow: &WorkflowFile) -> GraphData<'_> {
    let mut node_ids = BTreeSet::new();
    let mut node_definitions = HashMap::new();
    for node in &workflow.nodes {
        node_ids.insert(node.id.clone());
        node_definitions.insert(node.id.clone(), node);
    }

    let mut success_edges = Vec::new();
    let mut failure_by_from = BTreeMap::<String, String>::new();
    let mut join_indegree = HashMap::<String, usize>::new();
    for edge in &workflow.edges {
        node_ids.insert(edge.from.clone());
        node_ids.insert(edge.to.clone());
        node_ids.insert(edge.failure.clone());
        success_edges.push((edge.from.clone(), edge.to.clone()));
        *join_indegree.entry(edge.to.clone()).or_insert(0) += 1;
        failure_by_from
            .entry(edge.from.clone())
            .or_insert_with(|| edge.failure.clone());
    }
    success_edges.sort();

    let failure_edges = failure_by_from.into_iter().collect();

    GraphData {
        node_ids,
        node_definitions,
        success_edges,
        failure_edges,
        join_indegree,
    }
}

fn node_label(node_id: &str, node: Option<&&WorkflowNode>, indegree: Option<usize>) -> String {
    let mut label = match node {
        Some(node) => {
            let mut fields = vec![node_id.to_string(), node_kind_label(node.kind).to_string()];
            if let Some(name) = &node.name {
                fields.push(format!("name={name}"));
            }
            if let Some(count) = node.count {
                fields.push(format!("count={count}"));
            }
            if let Some(ends_loop) = &node.ends_loop {
                fields.push(format!("loop={ends_loop}"));
            }
            fields.join(" | ")
        }
        None => node_id.to_string(),
    };
    if let Some(d) = indegree {
        if d > 1 {
            label.push_str(&format!(" | join={d}"));
        }
    }
    label
}

fn node_kind_label(kind: NodeKind) -> &'static str {
    match kind {
        NodeKind::Task => "task",
        NodeKind::Start => "start",
        NodeKind::End => "end",
        NodeKind::Abort => "abort",
        NodeKind::Loop => "loop",
        NodeKind::LoopEnd => "loop_end",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{WorkflowEdge, WorkflowNode};

    fn sample_workflow() -> WorkflowFile {
        WorkflowFile {
            nodes: vec![
                WorkflowNode {
                    id: "start".into(),
                    kind: NodeKind::Start,
                    name: None,
                    count: None,
                    ends_loop: None,
                },
                WorkflowNode {
                    id: "a".into(),
                    kind: NodeKind::Task,
                    name: Some("A".into()),
                    count: None,
                    ends_loop: None,
                },
                WorkflowNode {
                    id: "b".into(),
                    kind: NodeKind::Task,
                    name: Some("B".into()),
                    count: None,
                    ends_loop: None,
                },
                WorkflowNode {
                    id: "join".into(),
                    kind: NodeKind::Task,
                    name: None,
                    count: None,
                    ends_loop: None,
                },
            ],
            edges: vec![
                WorkflowEdge {
                    from: "start".into(),
                    to: "a".into(),
                    failure: "abort".into(),
                },
                WorkflowEdge {
                    from: "start".into(),
                    to: "b".into(),
                    failure: "abort".into(),
                },
                WorkflowEdge {
                    from: "a".into(),
                    to: "join".into(),
                    failure: "abort".into(),
                },
                WorkflowEdge {
                    from: "b".into(),
                    to: "join".into(),
                    failure: "abort".into(),
                },
            ],
        }
    }

    #[test]
    fn mermaid_contains_expected_nodes_and_edges() {
        let out = render(&sample_workflow(), WorkflowVizFormat::Mermaid);
        assert!(out.contains("flowchart TD"));
        assert!(out.contains("failure"));
        assert!(out.contains("-->"));
        assert!(out.contains("join=2"));
    }

    #[test]
    fn ascii_lists_success_and_failure_sections() {
        let out = render(&sample_workflow(), WorkflowVizFormat::Ascii);
        assert!(out.contains("Nodes"));
        assert!(out.contains("Success edges"));
        assert!(out.contains("Failure edges"));
        assert!(out.contains("- start -> a"));
        assert!(out.contains("- start -x-> abort"));
    }

    #[test]
    fn mermaid_reserved_ids_are_prefixed() {
        let wf = WorkflowFile {
            nodes: vec![
                WorkflowNode {
                    id: "flowchart".into(),
                    kind: NodeKind::Task,
                    name: None,
                    count: None,
                    ends_loop: None,
                },
                WorkflowNode {
                    id: "subgraph".into(),
                    kind: NodeKind::Task,
                    name: None,
                    count: None,
                    ends_loop: None,
                },
                WorkflowNode {
                    id: "end".into(),
                    kind: NodeKind::End,
                    name: None,
                    count: None,
                    ends_loop: None,
                },
            ],
            edges: vec![],
        };
        let out = render(&wf, WorkflowVizFormat::Mermaid);
        assert!(out.contains("n_flowchart["));
        assert!(out.contains("n_subgraph["));
        assert!(out.contains("n_end["));
    }
}
