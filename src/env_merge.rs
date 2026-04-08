use std::collections::HashMap;

use crate::config::EnvStrategy;
use crate::config::EnvEntry;

pub fn merge_entries(base: HashMap<String, String>, entries: &[EnvEntry]) -> HashMap<String, String> {
    let mut env = base;
    for e in entries {
        apply_entry(&mut env, e);
    }
    env
}

fn apply_entry(env: &mut HashMap<String, String>, e: &EnvEntry) {
    let sep = e.separator.as_deref().unwrap_or(":");
    match e.strategy {
        EnvStrategy::Override => {
            env.insert(e.name.clone(), e.value.clone());
        }
        EnvStrategy::Prepend => {
            let new_val = match env.get(&e.name) {
                Some(old) if !old.is_empty() => format!("{}{}{}", e.value, sep, old),
                _ => e.value.clone(),
            };
            env.insert(e.name.clone(), new_val);
        }
        EnvStrategy::Append => {
            let new_val = match env.get(&e.name) {
                Some(old) if !old.is_empty() => format!("{}{}{}", old, sep, e.value),
                _ => e.value.clone(),
            };
            env.insert(e.name.clone(), new_val);
        }
    }
}
