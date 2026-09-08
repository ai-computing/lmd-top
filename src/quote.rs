//! Validation for model identifiers arriving from users and from the network.
//!
//! The Zoo view pulls model ids straight off the Hugging Face API, so these strings are remote
//! input, not just the operator's own typing, and they end up naming Kubernetes objects and
//! store paths. Rejecting a malformed id at the boundary is better than carrying it further:
//! manifests are serialized (see `crate::manifest`), so a hostile value can no longer become
//! YAML or shell syntax, but a string that is not a model id is still not something to deploy.
//!
//! This used to also hold `yamlq`/`shq` escaping helpers. Serializing the manifests removed
//! every caller — there is no longer a place where a value is spliced into text.

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

