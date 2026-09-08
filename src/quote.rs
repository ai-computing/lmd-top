//! Escaping and validation for values interpolated into generated manifests.
//!
//! Manifests are assembled as text, so every user- or network-sourced string crosses two
//! boundaries: YAML (a double-quoted scalar) and, for `sh -c` jobs, the shell. Before this
//! module both were unescaped — a model id with `"` or a newline produced a manifest that would
//! not parse, and one with `;` injected commands into the compile Job's command line (BUG-06).
//! The Zoo view pulls model ids straight off the Hugging Face API, so those strings are remote
//! input, not just the operator's own typing.
//!
//! Three layers, in order of preference:
//!   1. `valid_model_id` — reject malformed ids at the boundary (ingestion + form submit).
//!   2. `yamlq` — escape whatever reaches a YAML double-quoted scalar.
//!   3. `shq` — single-quote whatever reaches a shell word.

/// Escape a string for a YAML double-quoted scalar (the `"…"` is *not* included).
/// Backslash and quote are escaped; control characters become escapes so a newline can never
/// end the scalar early and turn the rest of the value into stray YAML.
pub fn yamlq(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 8);
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 || c as u32 == 0x7f => {
                out.push_str(&format!("\\x{:02x}", c as u32))
            }
            c => out.push(c),
        }
    }
    out
}

/// Quote a string as a single POSIX shell word. Single quotes protect everything;
/// an embedded `'` is closed, escaped, and reopened (`'\''`).
pub fn shq(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('\'');
    for c in s.chars() {
        if c == '\'' {
            out.push_str("'\\''");
        } else {
            out.push(c);
        }
    }
    out.push('\'');
    out
}

/// Is this a Hugging Face repo id (`name` or `org/name`) or an absolute local path?
///
/// Both are what the compile/deploy paths actually accept: the id goes to
/// `from_pretrained()` / `fxb build`, or names a directory in the model store. Anything else
/// is a typo or an injection attempt, and is far better rejected than escaped.
pub fn valid_model_id(s: &str) -> bool {
    if s.is_empty() || s.len() > 256 {
        return false;
    }
    if let Some(path) = s.strip_prefix('/') {
        // Local store path — plain path segments only, no traversal.
        return !path.is_empty()
            && path.split('/').all(|seg| {
                !seg.is_empty()
                    && seg != "."
                    && seg != ".."
                    && seg
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
            });
    }
    let seg_ok = |seg: &str| {
        !seg.is_empty()
            && seg != "."
            && seg != ".."
            && seg
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
    };
    let mut parts = s.split('/');
    match (parts.next(), parts.next(), parts.next()) {
        (Some(a), None, _) => seg_ok(a),
        (Some(a), Some(b), None) => seg_ok(a) && seg_ok(b),
        _ => false, // more than one '/' is not an HF repo id
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn yamlq_escapes_quote_and_newline() {
        assert_eq!(yamlq("plain"), "plain");
        assert_eq!(yamlq("a\"b"), "a\\\"b");
        assert_eq!(yamlq("a\\b"), "a\\\\b");
        assert_eq!(yamlq("a\nb"), "a\\nb");
        assert_eq!(yamlq("a\u{1}b"), "a\\x01b");
        // CJK must survive untouched (it is legal in a double-quoted scalar).
        assert_eq!(yamlq("모델-x"), "모델-x");
    }

    #[test]
    fn yamlq_output_parses_as_yaml() {
        for raw in [
            "Qwen/x\"-0.5B",
            "Qwen/x\nbar",
            "a: b # c",
            "{{tpl}}",
            "back\\slash",
        ] {
            let doc = format!("value: \"{}\"\n", yamlq(raw));
            let v: serde_yaml::Value = serde_yaml::from_str(&doc)
                .unwrap_or_else(|e| panic!("{:?} → invalid YAML: {}", raw, e));
            assert_eq!(v["value"].as_str(), Some(raw), "round-trip for {:?}", raw);
        }
    }

    #[test]
    fn shq_neutralises_command_separators() {
        assert_eq!(shq("plain"), "'plain'");
        assert_eq!(shq("a; echo PWNED; #"), "'a; echo PWNED; #'");
        assert_eq!(shq("it's"), "'it'\\''s'");
        // Quoted form contains no unquoted metacharacter.
        let q = shq("$(id); `id`; a&b|c>d");
        assert!(q.starts_with('\'') && q.ends_with('\''));
        assert_eq!(q.matches('\'').count(), 2);
    }

    #[test]
    fn valid_model_id_accepts_real_ids() {
        for ok in [
            "Qwen/Qwen2.5-0.5B-Instruct",
            "meta-llama/Llama-3.1-8B-Instruct",
            "furiosa-ai/Qwen3-4B-FP8",
            "LGAI-EXAONE/EXAONE-4.0-32B",
            "gpt2",
            "/mnt/store/compiled/Qwen--Qwen3-4B/furiosa/rngd-tp8",
        ] {
            assert!(valid_model_id(ok), "should accept {:?}", ok);
        }
    }

    #[test]
    fn valid_model_id_rejects_injection() {
        for bad in [
            "",
            "Qwen/x\"-0.5B",              // breaks the YAML scalar
            "Qwen/x\nbar",                // breaks the document
            "Qwen/x; echo PWNED; #",      // shell injection
            "Qwen/$(id)",                 // command substitution
            "Qwen/x y",                   // space splits a shell word
            "a/b/c",                      // not an HF repo id
            "../../etc/passwd",           // traversal
            "/mnt/store/../../etc/shadow",
            "Qwen/큐원",                  // non-ASCII is not a valid HF repo id
        ] {
            assert!(!valid_model_id(bad), "should reject {:?}", bad);
        }
    }
}
