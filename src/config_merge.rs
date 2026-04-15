use crate::config::{ConfigBundle, EnvEntry, EnvStrategy, NodeKind, TransferSpec, WorkflowNode};

pub fn merge_bundle_to_toml(bundle: &ConfigBundle) -> String {
    let mut out = String::new();
    write_servers(&mut out, bundle);
    write_shells(&mut out, bundle);
    write_commands(&mut out, bundle);
    write_tasks(&mut out, bundle);
    write_nodes(&mut out, bundle);
    write_edges(&mut out, bundle);
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out
}

fn write_servers(out: &mut String, bundle: &ConfigBundle) {
    let mut items: Vec<_> = bundle.servers.values().collect();
    items.sort_by(|a, b| a.id.cmp(&b.id));
    for s in items {
        out.push_str("[[servers]]\n");
        write_kv_string(out, "id", &s.id);
        write_kv_string(out, "kind", &s.kind);
        write_kv_opt_string(out, "description", s.description.as_deref());
        write_kv_opt_string(out, "transport", s.transport.as_deref());
        write_kv_opt_string(out, "host", s.host.as_deref());
        write_kv_opt_u16(out, "port", s.port);
        write_kv_opt_string(out, "user", s.user.as_deref());
        write_kv_opt_u64(out, "timeout", s.timeout);
        write_kv_opt_string(out, "password", s.password.as_deref());
        write_kv_opt_string(out, "password_env", s.password_env.as_deref());
        out.push('\n');
    }
}

fn write_shells(out: &mut String, bundle: &ConfigBundle) {
    let mut items: Vec<_> = bundle.shells.values().collect();
    items.sort_by(|a, b| a.id.cmp(&b.id));
    for s in items {
        out.push_str("[[shells]]\n");
        write_kv_string(out, "id", &s.id);
        write_kv_string(out, "program", &s.program);
        if !s.args.is_empty() {
            write_kv_string_array(out, "args", &s.args);
        }
        write_kv_opt_string(out, "description", s.description.as_deref());
        write_kv_opt_u64(out, "timeout", s.timeout);
        write_env_entries(out, "shells", &s.env);
        out.push('\n');
    }
}

fn write_commands(out: &mut String, bundle: &ConfigBundle) {
    let mut items: Vec<_> = bundle.commands.values().collect();
    items.sort_by(|a, b| a.id.cmp(&b.id));
    for c in items {
        out.push_str("[[commands]]\n");
        write_kv_string(out, "id", &c.id);
        write_kv_string(out, "command", &c.command);
        write_kv_opt_string(out, "description", c.description.as_deref());
        write_kv_opt_string(out, "cwd", c.cwd.as_deref());
        write_kv_opt_u64(out, "timeout", c.timeout);
        write_env_entries(out, "commands", &c.env);
        out.push('\n');
    }
}

fn write_tasks(out: &mut String, bundle: &ConfigBundle) {
    let mut items: Vec<_> = bundle.tasks.values().collect();
    items.sort_by(|a, b| a.id.cmp(&b.id));
    for t in items {
        out.push_str("[[tasks]]\n");
        write_kv_string(out, "id", &t.id);
        match &t.transfer {
            Some(tr) => {
                write_transfer_inline(out, tr);
            }
            None => {
                write_kv_opt_string(out, "server_id", t.server_id.as_deref());
                write_kv_opt_string(out, "shell_id", t.shell_id.as_deref());
                write_kv_opt_string(out, "command_id", t.command_id.as_deref());
            }
        }
        write_kv_opt_string(out, "description", t.description.as_deref());
        write_kv_opt_u64(out, "timeout", t.timeout);
        if t.retry != 0 {
            write_kv_u32(out, "retry", t.retry);
        }
        write_env_entries(out, "tasks", &t.env);
        out.push('\n');
    }
}

fn write_nodes(out: &mut String, bundle: &ConfigBundle) {
    let mut items: Vec<_> = bundle.workflow.nodes.iter().collect();
    items.sort_by(|a, b| a.id.cmp(&b.id));
    for n in items {
        if is_implicit_default_control_node(bundle, n) {
            continue;
        }
        if is_implicit_task_node(bundle, n) {
            continue;
        }
        out.push_str("[[nodes]]\n");
        write_kv_string(out, "id", &n.id);
        if !matches!(n.kind, NodeKind::Task) {
            write_kv_string(out, "type", node_kind_name(n.kind));
        }
        write_kv_opt_string(out, "name", n.name.as_deref());
        if let Some(count) = n.count {
            write_kv_u32(out, "count", count);
        }
        write_kv_opt_string(out, "loop", n.ends_loop.as_deref());
        out.push('\n');
    }
}

fn write_edges(out: &mut String, bundle: &ConfigBundle) {
    for e in &bundle.workflow.edges {
        out.push_str("[[edges]]\n");
        write_kv_string(out, "from", &e.from);
        write_kv_string(out, "to", &e.to);
        if e.failure != "abort" {
            write_kv_string(out, "failure", &e.failure);
        }
        out.push('\n');
    }
}

fn write_env_entries(out: &mut String, parent: &str, envs: &[EnvEntry]) {
    for e in envs {
        out.push_str(&format!("[[{parent}.env]]\n"));
        write_kv_string(out, "name", &e.name);
        write_kv_string(out, "strategy", env_strategy_name(&e.strategy));
        write_kv_string(out, "value", &e.value);
        write_kv_opt_string(out, "separator", e.separator.as_deref());
    }
}

fn write_transfer_inline(out: &mut String, tr: &TransferSpec) {
    out.push_str("transfer = { ");
    out.push_str("source_server_id = ");
    out.push_str(&toml_string(&tr.source_server_id));
    out.push_str(", dest_server_id = ");
    out.push_str(&toml_string(&tr.dest_server_id));
    out.push_str(", source_path = ");
    out.push_str(&toml_string(&tr.source_path));
    out.push_str(", dest_path = ");
    out.push_str(&toml_string(&tr.dest_path));
    out.push_str(" }\n");
}

fn is_implicit_default_control_node(bundle: &ConfigBundle, n: &WorkflowNode) -> bool {
    matches!(n.id.as_str(), "start" | "end" | "abort")
        && !bundle.explicit_control_nodes.contains(n.id.as_str())
}

fn is_implicit_task_node(bundle: &ConfigBundle, n: &WorkflowNode) -> bool {
    bundle.tasks.contains_key(&n.id)
        && matches!(n.kind, NodeKind::Task)
        && n.name.is_none()
        && n.count.is_none()
        && n.ends_loop.is_none()
        && !bundle.explicit_task_nodes.contains(&n.id)
}

fn node_kind_name(kind: NodeKind) -> &'static str {
    match kind {
        NodeKind::Task => "task",
        NodeKind::Start => "start",
        NodeKind::End => "end",
        NodeKind::Abort => "abort",
        NodeKind::Loop => "loop",
        NodeKind::LoopEnd => "loop_end",
    }
}

fn env_strategy_name(s: &EnvStrategy) -> &'static str {
    match s {
        EnvStrategy::Override => "override",
        EnvStrategy::Prepend => "prepend",
        EnvStrategy::Append => "append",
    }
}

fn write_kv_string(out: &mut String, key: &str, value: &str) {
    out.push_str(key);
    out.push_str(" = ");
    out.push_str(&toml_string(value));
    out.push('\n');
}

fn write_kv_opt_string(out: &mut String, key: &str, value: Option<&str>) {
    if let Some(v) = value {
        write_kv_string(out, key, v);
    }
}

fn write_kv_opt_u64(out: &mut String, key: &str, value: Option<u64>) {
    if let Some(v) = value {
        out.push_str(key);
        out.push_str(" = ");
        out.push_str(&v.to_string());
        out.push('\n');
    }
}

fn write_kv_opt_u16(out: &mut String, key: &str, value: Option<u16>) {
    if let Some(v) = value {
        out.push_str(key);
        out.push_str(" = ");
        out.push_str(&v.to_string());
        out.push('\n');
    }
}

fn write_kv_u32(out: &mut String, key: &str, value: u32) {
    out.push_str(key);
    out.push_str(" = ");
    out.push_str(&value.to_string());
    out.push('\n');
}

fn write_kv_string_array(out: &mut String, key: &str, values: &[String]) {
    let body = values
        .iter()
        .map(|v| toml_string(v))
        .collect::<Vec<_>>()
        .join(", ");
    out.push_str(key);
    out.push_str(" = [");
    out.push_str(&body);
    out.push_str("]\n");
}

fn toml_string(s: &str) -> String {
    toml::Value::String(s.to_string()).to_string()
}

