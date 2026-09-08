//! Manifest construction — build Kubernetes objects as **data**, then serialize.
//!
//! Manifests used to be assembled with `format!` over YAML text. That made correctness the
//! author's problem at every call site: interpolated values had to be escaped by hand (a model
//! id containing `"` produced an unparseable manifest, one containing `;` injected shell
//! commands — BUG-06), and indentation was encoded as 181 `\x20` escapes.
//!
//! Here the object tree is built with [`ymap!`] and serialized by `serde_yaml`, so quoting is
//! the serializer's job and injection is not expressible. Operator-facing comments — which are
//! part of this tool's value, since a human reads the preview before applying — survive as an
//! explicit preamble rather than being interleaved into the text.

use serde_yaml::{Mapping, Value};

/// Build a YAML mapping, preserving key order (`serde_yaml::Mapping` is insertion-ordered, so
/// output keeps the conventional apiVersion/kind/metadata/spec shape rather than sorting).
///
/// ```ignore
/// ymap! {
///     "apiVersion" => s("apps/v1"),
///     "kind" => s("Deployment"),
///     "metadata" => ymap! { "name" => s(name), "namespace" => s(ns) },
/// }
/// ```
#[macro_export]
macro_rules! ymap {
    ($($k:expr => $v:expr),* $(,)?) => {{
        #[allow(unused_mut)]
        let mut m = serde_yaml::Mapping::new();
        $( m.insert(serde_yaml::Value::from($k), $v); )*
        serde_yaml::Value::Mapping(m)
    }};
}

/// String scalar. Named for brevity — manifests are mostly strings, and `s("x")` at the call
/// site reads better than `Value::from("x".to_string())` repeated a few hundred times.
pub fn s(v: impl Into<String>) -> Value {
    Value::String(v.into())
}

/// Sequence node.
pub fn seq(items: Vec<Value>) -> Value {
    Value::Sequence(items)
}

/// `command: ["sh", "-c", ...]` style argument vector.
pub fn args<I, T>(items: I) -> Value
where
    I: IntoIterator<Item = T>,
    T: Into<String>,
{
    seq(items.into_iter().map(s).collect())
}

/// `{ name, value }` container environment entry.
pub fn env_val(name: &str, value: impl Into<String>) -> Value {
    ymap! { "name" => s(name), "value" => s(value) }
}

/// `{ name, valueFrom: { secretKeyRef: … } }` container environment entry.
pub fn env_secret(name: &str, secret: &str, key: &str, optional: bool) -> Value {
    ymap! {
        "name" => s(name),
        "valueFrom" => ymap! {
            "secretKeyRef" => ymap! {
                "name" => s(secret),
                "key" => s(key),
                "optional" => Value::Bool(optional),
            },
        },
    }
}

/// `{ name, mountPath[, readOnly] }` volume mount.
pub fn mount(name: &str, path: &str, read_only: bool) -> Value {
    let mut m = Mapping::new();
    m.insert("name".into(), s(name));
    m.insert("mountPath".into(), s(path));
    if read_only {
        m.insert("readOnly".into(), Value::Bool(true));
    }
    Value::Mapping(m)
}

/// One document of a multi-document manifest, with the comment lines that precede it.
pub struct Doc {
    pub comments: Vec<String>,
    pub body: Value,
}

impl Doc {
    pub fn new(body: Value) -> Self {
        Doc {
            comments: Vec::new(),
            body,
        }
    }

    /// Attach an explanatory comment. Text is emitted verbatim after `# `, so it must not
    /// contain newlines — call this once per line.
    pub fn note(mut self, line: impl Into<String>) -> Self {
        self.comments.push(line.into());
        self
    }
}

/// A complete manifest: leading commentary plus the documents to apply.
#[derive(Default)]
pub struct Manifest {
    pub preamble: Vec<String>,
    pub docs: Vec<Doc>,
}

impl Manifest {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a top-of-file comment line (context for whoever reviews the preview).
    pub fn note(mut self, line: impl Into<String>) -> Self {
        self.preamble.push(line.into());
        self
    }

    pub fn push(mut self, doc: Doc) -> Self {
        self.docs.push(doc);
        self
    }

    /// Render to YAML: preamble comments, then `---`-separated documents.
    pub fn to_yaml(&self) -> String {
        let mut out = String::new();
        for line in &self.preamble {
            out.push_str("# ");
            out.push_str(line);
            out.push('\n');
        }
        for (i, doc) in self.docs.iter().enumerate() {
            if i > 0 {
                out.push_str("---\n");
            }
            for line in &doc.comments {
                out.push_str("# ");
                out.push_str(line);
                out.push('\n');
            }
            match serde_yaml::to_string(&doc.body) {
                Ok(text) => out.push_str(&text),
                // Unreachable for the Value trees built here (no NaN keys, no cycles); degrade
                // to a visible marker rather than silently emitting a truncated manifest.
                Err(e) => out.push_str(&format!("# ERROR: could not serialize document: {}\n", e)),
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_order_follows_insertion() {
        let doc = ymap! {
            "apiVersion" => s("apps/v1"),
            "kind" => s("Deployment"),
            "metadata" => ymap! { "name" => s("x") },
        };
        let out = serde_yaml::to_string(&doc).unwrap();
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines[0], "apiVersion: apps/v1");
        assert_eq!(lines[1], "kind: Deployment");
    }

    /// The whole point of Phase 1: a hostile value is data, not syntax. Anything that used to
    /// break out of the scalar (quote, newline, `#`, shell metacharacters) now round-trips.
    #[test]
    fn hostile_values_cannot_escape_the_scalar() {
        for hostile in [
            "Qwen/x\"; echo PWNED; #",
            "Qwen/foo\nbar: injected",
            "--- \nkind: Evil",
            "{{template}}",
            "back\\slash",
            "모델-이름",
        ] {
            let m = Manifest::new().push(Doc::new(ymap! {
                "apiVersion" => s("v1"),
                "kind" => s("ConfigMap"),
                "data" => ymap! { "model" => s(hostile) },
            }));
            let text = m.to_yaml();
            let docs: Vec<Value> = serde_yaml::Deserializer::from_str(&text)
                .map(|d| <Value as serde::Deserialize>::deserialize(d).expect("valid YAML"))
                .collect();
            assert_eq!(docs.len(), 1, "{:?} must not add documents", hostile);
            assert_eq!(
                docs[0]["data"]["model"].as_str(),
                Some(hostile),
                "{:?} must round-trip as a value",
                hostile
            );
            assert_eq!(docs[0]["kind"].as_str(), Some("ConfigMap"));
        }
    }

    #[test]
    fn multi_doc_output_separates_and_comments() {
        let m = Manifest::new()
            .note("preview — review before applying")
            .push(Doc::new(ymap! { "kind" => s("ConfigMap") }).note("script"))
            .push(Doc::new(ymap! { "kind" => s("Job") }).note("the job"));
        let text = m.to_yaml();
        assert!(text.starts_with("# preview — review before applying\n"));
        assert!(text.contains("\n---\n"));
        let kinds: Vec<String> = serde_yaml::Deserializer::from_str(&text)
            .map(|d| <Value as serde::Deserialize>::deserialize(d).unwrap())
            .filter_map(|v| v["kind"].as_str().map(String::from))
            .collect();
        assert_eq!(kinds, vec!["ConfigMap", "Job"]);
    }
}
