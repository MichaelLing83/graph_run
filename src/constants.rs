//! Optional `${NAME}` expansion for config TOML (values from `--constants` file).

use std::collections::HashMap;
use std::fs;
use std::path::Path;

use crate::error::{GraphRunError, Result};

/// Load a TOML file of top-level key → scalar values (string, integer, float, or boolean).
/// Arrays and inline tables are rejected. Keys become `${KEY}` substitution names.
pub fn load_constants_file(path: &Path) -> Result<HashMap<String, String>> {
    let file = path.to_path_buf();
    let text = fs::read_to_string(path).map_err(|source| GraphRunError::Io {
        file: file.clone(),
        source,
    })?;
    let root: toml::Value = toml::from_str(&text).map_err(|source| GraphRunError::Toml {
        file: file.clone(),
        source,
    })?;
    let table = root.as_table().ok_or_else(|| {
        GraphRunError::msg(format!(
            "constants file {} must be a TOML table at the root (key = value rows)",
            path.display()
        ))
    })?;
    let mut out = HashMap::new();
    for (k, v) in table {
        let s = scalar_to_string(v).map_err(|e| {
            GraphRunError::msg(format!(
                "constants file {}: key {:?}: {}",
                path.display(),
                k,
                e
            ))
        })?;
        if out.insert(k.clone(), s).is_some() {
            return Err(GraphRunError::msg(format!(
                "constants file {}: duplicate key {:?}",
                path.display(),
                k
            )));
        }
    }
    Ok(out)
}

fn scalar_to_string(v: &toml::Value) -> std::result::Result<String, &'static str> {
    match v {
        toml::Value::String(s) => Ok(s.clone()),
        toml::Value::Integer(i) => Ok(i.to_string()),
        toml::Value::Float(f) => Ok(f.to_string()),
        toml::Value::Boolean(b) => Ok(b.to_string()),
        toml::Value::Datetime(d) => Ok(d.to_string()),
        _ => Err("value must be a string, number, boolean, or datetime (not an array or table)"),
    }
}

/// Replace every `${IDENT}` with `constants[IDENT]`. `IDENT` is `[A-Za-z0-9_]+`.
/// Unknown names and unclosed `${` are errors. Literal `$` not followed by `{` is unchanged.
pub(crate) fn expand_template(text: &str, constants: &HashMap<String, String>, path: &Path) -> Result<String> {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(pos) = rest.find("${") {
        out.push_str(&rest[..pos]);
        rest = &rest[pos + 2..];
        let end = rest.find('}').ok_or_else(|| {
            GraphRunError::msg(format!(
                "unclosed `${{...}}` placeholder in {}",
                path.display()
            ))
        })?;
        let name = &rest[..end];
        if name.is_empty() {
            return Err(GraphRunError::msg(format!(
                "empty `${{}}` placeholder in {}",
                path.display()
            )));
        }
        if !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_')
        {
            return Err(GraphRunError::msg(format!(
                "invalid placeholder `${{{name}}}` in {} (use [A-Za-z0-9_] only)",
                path.display()
            )));
        }
        let val = constants.get(name).ok_or_else(|| {
            GraphRunError::msg(format!(
                "unknown constant `{name}` referenced in {}",
                path.display()
            ))
        })?;
        out.push_str(val);
        rest = &rest[end + 1..];
    }
    out.push_str(rest);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn expand_replaces_placeholders() {
        let mut m = HashMap::new();
        m.insert("HOST".into(), "10.0.0.1".into());
        m.insert("PORT".into(), "22".into());
        let p = Path::new("test.toml");
        let s = expand_template(
            r#"host = "${HOST}", port = ${PORT}"#,
            &m,
            p,
        )
        .unwrap();
        assert_eq!(s, r#"host = "10.0.0.1", port = 22"#);
    }

    #[test]
    fn expand_unknown_errors() {
        let m = HashMap::new();
        let p = Path::new("x.toml");
        let e = expand_template("x = ${MISSING}", &m, p).unwrap_err();
        let msg = e.to_string();
        assert!(msg.contains("MISSING"), "{msg}");
    }

    #[test]
    fn expand_unclosed_errors() {
        let mut m = HashMap::new();
        m.insert("A".into(), "1".into());
        let p = Path::new("x.toml");
        assert!(expand_template("x = ${A", &m, p).is_err());
    }
}
