//! Failing pods, why they fail, and images whose tag no longer means one thing.
//!
//! A CrashLoopBackOff used to be a restart counter: the reason sat in a log nobody opened, and
//! the 2026-10-14 incidents each took a manual `kubectl logs --previous` to explain — an EPP
//! image rebuilt under the floating `:main` tag that rejected its config's apiVersion, a
//! config-reloader out of inotify instances, an amd64-only operator scheduled on an arm64 node.
//! Each is one line in the log and one known fix. This module finds failing containers in the
//! cluster-wide pod list the inventory already reads, picks that line, and names the fix.
//!
//! The cause comes, in order of cost, from the waiting message (image pulls, config errors),
//! the container's termination message (many system pods publish their log tail there), and
//! only then from `logs --previous`, fetched by the collector for a few pods per cycle.
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

/// Waiting reasons that mean the container will not run without someone changing something.
const BAD_WAITING: &[&str] = &[
    "CrashLoopBackOff",
    "ImagePullBackOff",
    "ErrImagePull",
    "InvalidImageName",
    "CreateContainerConfigError",
    "CreateContainerError",
    "RunContainerError",
];

/// A container that restarted within this window is still failing, even if it is `Running`
/// at the instant we look (a crash loop spends most of its time between back-offs).
const RECENT_RESTART_SECS: u64 = 600;

/// Tags that are re-pointed by their publisher, so the same manifest can run different code.
const FLOATING_TAGS: &[&str] = &[
    "latest", "main", "master", "nightly", "dev", "edge", "stable", "head",
];

/// What the node a pod runs on looks like, from the node list.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct NodeFacts {
    pub arch: String,
    pub ready: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct PodProblem {
    pub namespace: String,
    pub pod: String,
    pub container: String,
    /// Init container: the pod shows `Init:CrashLoopBackOff` and never starts its main one.
    pub init: bool,
    pub node: String,
    /// `CrashLoopBackOff`, `ImagePullBackOff`, `OOMKilled`, `restarting`, …
    pub state: String,
    pub restarts: i64,
    pub exit_code: Option<i64>,
    pub image: String,
    /// The image's tag is one publishers move (`:latest`, `:main`, none at all).
    pub floating_tag: Option<String>,
    /// The most informative log or status line; `None` until one has been read.
    pub cause: Option<String>,
    /// What to change, when the cause implies it.
    pub hint: Option<String>,
    /// Restart count the cause was read at, so a cached log line is not reused for a new crash.
    #[serde(skip)]
    pub cause_at_restart: i64,
}

impl PodProblem {
    /// Whether the collector should fetch `logs --previous` for this container.
    pub fn wants_log(&self, nodes: &BTreeMap<String, NodeFacts>) -> bool {
        self.cause.is_none()
            && matches!(
                self.state.as_str(),
                "CrashLoopBackOff" | "restarting" | "Error"
            )
            && nodes.get(&self.node).is_some_and(|n| n.ready)
    }

    /// One line for status bands, alerts and snapshots.
    pub fn line(&self) -> String {
        let mut s = format!(
            "{}/{}{}: {}",
            self.namespace,
            self.pod,
            if self.init { " (init)" } else { "" },
            self.state
        );
        if self.restarts > 0 {
            s.push_str(&format!(" ×{}", self.restarts));
        }
        if let Some(cause) = &self.cause {
            s.push_str(&format!(" — {cause}"));
        }
        if let Some(hint) = &self.hint {
            s.push_str(&format!(" → {hint}"));
        }
        s
    }
}

/// One image reference running as more than one digest: the tag moved between pulls.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ImageDrift {
    pub image: String,
    pub digests: Vec<String>,
    /// `namespace/pod` per digest, in `digests` order.
    pub pods: Vec<Vec<String>>,
}

impl ImageDrift {
    pub fn line(&self) -> String {
        let counts: Vec<String> = self
            .digests
            .iter()
            .zip(&self.pods)
            .map(|(d, p)| format!("{}×{}", short_digest(d), p.len()))
            .collect();
        format!(
            "{} runs {} different builds ({}) — the tag moved; pin a digest",
            self.image,
            self.digests.len(),
            counts.join(", ")
        )
    }
}

fn short_digest(d: &str) -> &str {
    let hex = d.strip_prefix("sha256:").unwrap_or(d);
    &hex[..hex.len().min(12)]
}

/// Node name → facts, from a `kubectl get nodes -o json` list.
pub fn node_facts(nodes: &serde_json::Value) -> BTreeMap<String, NodeFacts> {
    let mut out = BTreeMap::new();
    for n in nodes["items"].as_array().into_iter().flatten() {
        let Some(name) = n["metadata"]["name"].as_str() else {
            continue;
        };
        let ready = n["status"]["conditions"]
            .as_array()
            .into_iter()
            .flatten()
            .any(|c| c["type"] == "Ready" && c["status"] == "True");
        let arch = n["metadata"]["labels"]["kubernetes.io/arch"]
            .as_str()
            .or_else(|| n["status"]["nodeInfo"]["architecture"].as_str())
            .unwrap_or("")
            .to_string();
        out.insert(name.to_string(), NodeFacts { arch, ready });
    }
    out
}

/// The tag a publisher moves, if `image` uses one. Digest references are pinned by definition.
pub fn floating_tag(image: &str) -> Option<String> {
    if image.contains('@') || image.starts_with("sha256:") {
        return None;
    }
    let last = image.rsplit('/').next().unwrap_or(image);
    match last.rsplit_once(':') {
        None => Some("latest".into()),
        Some((_, tag)) if FLOATING_TAGS.contains(&tag) => Some(tag.into()),
        Some(_) => None,
    }
}

/// Failing containers in a `kubectl get pods -A -o json` list.
pub fn problems(
    pods: &serde_json::Value,
    nodes: &BTreeMap<String, NodeFacts>,
    now: u64,
) -> Vec<PodProblem> {
    let mut out = Vec::new();
    for p in pods["items"].as_array().into_iter().flatten() {
        let namespace = p["metadata"]["namespace"].as_str().unwrap_or("");
        let pod = p["metadata"]["name"].as_str().unwrap_or("");
        let node = p["spec"]["nodeName"].as_str().unwrap_or("");
        for (key, init) in [
            ("initContainerStatuses", true),
            ("containerStatuses", false),
        ] {
            for c in p["status"][key].as_array().into_iter().flatten() {
                let Some(mut problem) = container_problem(c, now) else {
                    continue;
                };
                problem.namespace = namespace.into();
                problem.pod = pod.into();
                problem.node = node.into();
                problem.init = init;
                problem.hint = hint(&problem, nodes.get(node));
                out.push(problem);
            }
        }
    }
    // Actionable first: a container failing on a healthy node is a fix to make; one on a
    // NotReady node is a symptom of the node.
    out.sort_by_key(|p| {
        (
            !nodes.get(&p.node).is_some_and(|n| n.ready),
            p.namespace.clone(),
            p.pod.clone(),
        )
    });
    out
}

fn container_problem(c: &serde_json::Value, now: u64) -> Option<PodProblem> {
    let waiting = &c["state"]["waiting"];
    let last = &c["lastState"]["terminated"];
    let restarts = c["restartCount"].as_i64().unwrap_or(0);
    let waiting_reason = waiting["reason"].as_str().unwrap_or("");
    let last_reason = last["reason"].as_str().unwrap_or("");
    let last_exit = last["exitCode"].as_i64();
    let recently_restarted = restarts > 0
        && last_exit.is_some_and(|code| code != 0)
        && last["finishedAt"]
            .as_str()
            .and_then(crate::collect::parse_k8s_ts)
            .is_some_and(|t| now.saturating_sub(t) <= RECENT_RESTART_SECS);
    let state = if BAD_WAITING.contains(&waiting_reason) {
        if waiting_reason == "CrashLoopBackOff" && last_reason == "OOMKilled" {
            "OOMKilled".to_string()
        } else {
            waiting_reason.to_string()
        }
    } else if recently_restarted {
        if last_reason == "OOMKilled" {
            "OOMKilled".into()
        } else {
            "restarting".into()
        }
    } else {
        return None;
    };
    let image = c["image"].as_str().unwrap_or("").to_string();
    // Pull and config errors explain themselves in the waiting message; crashes may carry
    // their log tail in the termination message.
    let cause = match state.as_str() {
        "OOMKilled" => Some(format!("OOMKilled (exit {})", last_exit.unwrap_or(137))),
        "CrashLoopBackOff" | "restarting" => last["message"].as_str().and_then(cause_line),
        _ => waiting["message"]
            .as_str()
            .filter(|m| !m.trim().is_empty())
            .map(|m| clip(m.trim())),
    };
    Some(PodProblem {
        namespace: String::new(),
        pod: String::new(),
        container: c["name"].as_str().unwrap_or("").to_string(),
        init: false,
        node: String::new(),
        state,
        restarts,
        exit_code: last_exit,
        floating_tag: floating_tag(&image),
        image,
        cause,
        hint: None,
        cause_at_restart: restarts,
    })
}

fn clip(s: &str) -> String {
    const MAX: usize = 180;
    if s.chars().count() > MAX {
        format!("{}…", s.chars().take(MAX).collect::<String>())
    } else {
        s.to_string()
    }
}

/// The line of a crashed container's log that says why it stopped.
///
/// Priority: a fatal/panic line, then the last error line (a Python traceback's final
/// exception line counts), then the last non-empty line. Structured `msg="…" error="…"` logs
/// are reduced to their message and error.
pub fn cause_line(log: &str) -> Option<String> {
    let lines: Vec<&str> = log
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    let is = |l: &&str, needles: &[&str]| {
        let lower = l.to_ascii_lowercase();
        needles.iter().any(|n| lower.contains(n))
    };
    let pick = lines
        .iter()
        .rev()
        .find(|l| {
            is(
                l,
                &["level=fatal", "\"level\":\"fatal\"", "fatal:", "panic:"],
            )
        })
        .or_else(|| {
            lines.iter().rev().find(|l| {
                is(
                    l,
                    &[
                        "level=error",
                        "\"level\":\"error\"",
                        "error:",
                        "error ",
                        "exception",
                        "exec format error",
                        "no space left",
                        "traceback",
                    ],
                ) && !l.starts_with("File \"")
            })
        })
        .or(lines.last())?;
    Some(clip(&structured(pick)))
}

/// `level=fatal msg="failed to start: …" subsys=daemon` → `failed to start: …`.
fn structured(line: &str) -> String {
    let field = |name: &str| -> Option<String> {
        let at = line.find(&format!("{name}="))? + name.len() + 1;
        let rest = &line[at..];
        if let Some(quoted) = rest.strip_prefix('"') {
            let mut out = String::new();
            let mut escaped = false;
            for ch in quoted.chars() {
                match (escaped, ch) {
                    (true, c) => {
                        out.push(c);
                        escaped = false
                    }
                    (false, '\\') => escaped = true,
                    (false, '"') => return Some(out),
                    (false, c) => out.push(c),
                }
            }
            Some(out)
        } else {
            Some(rest.split_whitespace().next().unwrap_or("").to_string())
        }
    };
    match (field("msg"), field("error")) {
        (Some(msg), Some(err)) if !msg.contains(&err) => format!("{msg}: {err}"),
        (Some(msg), _) if !msg.is_empty() => msg,
        _ => line.to_string(),
    }
}

/// The fix a known cause implies. Order matters: a node that is down explains everything on it.
pub fn hint(p: &PodProblem, node: Option<&NodeFacts>) -> Option<String> {
    if let Some(n) = node.filter(|n| !n.ready) {
        let _ = n;
        return Some(format!(
            "node {} is NotReady — fix the node, not the pod",
            p.node
        ));
    }
    let arch = node.map(|n| n.arch.as_str()).unwrap_or("");
    let text = p.cause.as_deref().unwrap_or("").to_ascii_lowercase();
    let floating = p
        .floating_tag
        .as_deref()
        .map(|t| format!(" (image tag :{t} is floating — the build may have changed under it)"))
        .unwrap_or_default();
    let has = |needles: &[&str]| needles.iter().any(|n| text.contains(n));
    let hint = if has(&["exec format error"]) {
        format!(
            "image has no {} build — pin nodeSelector kubernetes.io/arch or use a multi-arch image",
            if arch.is_empty() { "matching" } else { arch }
        )
    } else if has(&["no space left on device"]) && has(&["inotify", "fsnotify", "watch"]) {
        format!(
            "inotify instances exhausted on {} — raise fs.inotify.max_user_instances",
            p.node
        )
    } else if has(&["no space left on device"]) {
        format!("disk full on {}", p.node)
    } else if has(&["is registered for version", "no kind"]) && has(&["registered"]) {
        format!("config apiVersion not understood by this image{floating}")
    } else if has(&[
        "unrecognized arguments",
        "unknown flag",
        "flag provided but not defined",
        "unrecognized option",
    ]) {
        format!("a command-line flag was rejected by this image version{floating}")
    } else if p.state == "OOMKilled" {
        "raise the container memory limit or reduce the workload".into()
    } else if has(&["6443", "api-server", "apiserver"]) && has(&["timeout", "refused", "no route"])
    {
        format!(
            "{} cannot reach the Kubernetes API — check node network/CNI",
            p.node
        )
    } else if matches!(p.state.as_str(), "ImagePullBackOff" | "ErrImagePull") {
        if has(&["no match for platform", "no matching manifest"]) {
            format!("image has no {arch} build")
        } else if has(&["not found", "manifest unknown"]) {
            "tag does not exist in the registry".into()
        } else if has(&["unauthorized", "denied", "authentication"]) {
            "registry denied the pull — check imagePullSecrets".into()
        } else {
            format!(
                "pull keeps failing — is the registry reachable from {}?",
                p.node
            )
        }
    } else if !floating.is_empty() && p.cause.is_some() {
        floating
            .trim_start()
            .trim_start_matches('(')
            .trim_end_matches(')')
            .into()
    } else {
        return None;
    };
    Some(hint)
}

/// Floating image references whose running containers resolved to more than one digest.
pub fn drift(pods: &serde_json::Value) -> Vec<ImageDrift> {
    let mut by_image: BTreeMap<String, BTreeMap<String, BTreeSet<String>>> = BTreeMap::new();
    for p in pods["items"].as_array().into_iter().flatten() {
        let who = format!(
            "{}/{}",
            p["metadata"]["namespace"].as_str().unwrap_or(""),
            p["metadata"]["name"].as_str().unwrap_or("")
        );
        for c in p["status"]["containerStatuses"]
            .as_array()
            .into_iter()
            .flatten()
        {
            let image = c["image"].as_str().unwrap_or("");
            let Some(digest) = c["imageID"]
                .as_str()
                .and_then(|id| id.rsplit_once('@').map(|(_, d)| d))
                .filter(|d| !d.is_empty())
            else {
                continue;
            };
            if floating_tag(image).is_none() {
                continue;
            }
            by_image
                .entry(normalize(image))
                .or_default()
                .entry(digest.to_string())
                .or_default()
                .insert(who.clone());
        }
    }
    by_image
        .into_iter()
        .filter(|(_, digests)| digests.len() > 1)
        .map(|(image, digests)| ImageDrift {
            image,
            pods: digests
                .values()
                .map(|p| p.iter().cloned().collect())
                .collect(),
            digests: digests.into_keys().collect(),
        })
        .collect()
}

/// `furiosaai/x` and `docker.io/furiosaai/x:latest` are the same reference.
fn normalize(image: &str) -> String {
    let image = image.strip_prefix("docker.io/").unwrap_or(image);
    let image = image.strip_prefix("library/").unwrap_or(image);
    let last = image.rsplit('/').next().unwrap_or(image);
    if last.contains(':') {
        image.to_string()
    } else {
        format!("{image}:latest")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn ready(arch: &str) -> NodeFacts {
        NodeFacts {
            arch: arch.into(),
            ready: true,
        }
    }
    fn nodes() -> BTreeMap<String, NodeFacts> {
        BTreeMap::from([
            ("amd".to_string(), ready("amd64")),
            ("arm".to_string(), ready("arm64")),
            (
                "down".to_string(),
                NodeFacts {
                    arch: "arm64".into(),
                    ready: false,
                },
            ),
        ])
    }
    fn pod(
        ns: &str,
        name: &str,
        node: &str,
        key: &str,
        status: serde_json::Value,
    ) -> serde_json::Value {
        json!({"metadata":{"namespace":ns,"name":name},"spec":{"nodeName":node},
               "status":{key:[status]}})
    }
    fn crash(image: &str, restarts: i64, last: serde_json::Value) -> serde_json::Value {
        json!({"name":"c","image":image,"restartCount":restarts,
               "state":{"waiting":{"reason":"CrashLoopBackOff"}},"lastState":{"terminated":last}})
    }
    const NOW: u64 = 2_000_000_000;

    #[test]
    fn floating_tags_are_recognised_and_digests_are_pinned() {
        assert_eq!(
            floating_tag("ghcr.io/llm-d/epp:main").as_deref(),
            Some("main")
        );
        assert_eq!(
            floating_tag("furiosaai/furiosa-llm").as_deref(),
            Some("latest")
        );
        assert_eq!(floating_tag("registry:5000/x").as_deref(), Some("latest"));
        assert_eq!(floating_tag("furiosaai/furiosa-llm:2026.3.0"), None);
        assert_eq!(floating_tag("ghcr.io/llm-d/epp@sha256:c0d6"), None);
        assert_eq!(floating_tag("sha256:a226bca9"), None);
    }

    #[test]
    fn epp_rejecting_its_config_after_main_moved() {
        // 2026-10-14: `:main` rebuilt; every EPP crash-looped on its EndpointPickerConfig.
        let list = json!({"items":[pod("llm-serving","gemma4-rbln-epp-7d9","amd","containerStatuses",
            crash("ghcr.io/llm-d/llm-d-router-endpoint-picker-dev:main", 31, json!({"exitCode":1,"reason":"Error"})))]});
        let mut found = problems(&list, &nodes(), NOW);
        assert_eq!(found.len(), 1);
        let p = &mut found[0];
        assert!(
            p.wants_log(&nodes()),
            "no termination message — read the log"
        );
        p.cause = cause_line(
            "{\"level\":\"info\",\"msg\":\"starting\"}\n\
             {\"level\":\"error\",\"msg\":\"failed to load config\",\"error\":\"no kind \\\"EndpointPickerConfig\\\" is registered for version \\\"inference.networking.x-k8s.io/v1alpha1\\\"\"}\n",
        );
        p.hint = hint(p, nodes().get("amd"));
        assert_eq!(
            p.hint.as_deref(),
            Some("config apiVersion not understood by this image (image tag :main is floating — the build may have changed under it)")
        );
        assert!(p
            .line()
            .starts_with("llm-serving/gemma4-rbln-epp-7d9: CrashLoopBackOff ×31 — "));
    }

    #[test]
    fn config_reloader_out_of_inotify_instances_is_an_init_crash() {
        // 2026-10-14: gb10-node-2 had max_user_instances=128; prometheus never got past init.
        let list = json!({"items":[pod("monitoring","prometheus-0","arm","initContainerStatuses",
            crash("quay.io/prometheus-operator/prometheus-config-reloader:v0.86.0", 12, json!({"exitCode":1,
              "message":"level=info msg=\"Starting\"\nlevel=error msg=\"Failed to run\" err=\"create watcher: too many open files\"\nfailed to create inotify watcher: no space left on device"})))]});
        let found = problems(&list, &nodes(), NOW);
        assert!(found[0].init);
        assert_eq!(
            found[0].hint.as_deref(),
            Some("inotify instances exhausted on arm — raise fs.inotify.max_user_instances")
        );
        assert!(
            !found[0].wants_log(&nodes()),
            "termination message already explains it"
        );
    }

    #[test]
    fn amd64_only_image_on_an_arm64_node() {
        // 2026-10-14: rbln-npu-operator controller scheduled on gb10-node-2.
        let list = json!({"items":[pod("rbln-system","rbln-npu-operator-controller-manager-x","arm","containerStatuses",
            crash("docker.io/rebellions/rbln-npu-operator:v0.2.0", 40,
                  json!({"exitCode":255,"message":"exec /manager: exec format error"})))]});
        let found = problems(&list, &nodes(), NOW);
        assert_eq!(
            found[0].cause.as_deref(),
            Some("exec /manager: exec format error")
        );
        assert_eq!(
            found[0].hint.as_deref(),
            Some("image has no arm64 build — pin nodeSelector kubernetes.io/arch or use a multi-arch image")
        );
    }

    #[test]
    fn crash_on_a_notready_node_blames_the_node_and_sorts_last() {
        // 2026-10-19: cilium-agent on NotReady nodes cannot reach the API server.
        let msg = "time=\"x\" level=info msg=\"Establishing connection to apiserver\"\n\
                   time=\"x\" level=fatal msg=\"failed to start: Get \\\"https://192.0.2.1:6443/api/v1/namespaces/kube-system\\\": dial tcp 192.0.2.1:6443: i/o timeout\" subsys=daemon\n\
                   time=x level=error msg=\"Start hook failed\" error=\"Get ...\"";
        let list = json!({"items":[
            pod("kube-system","cilium-a","down","containerStatuses",
                crash("quay.io/cilium/cilium@sha256:01d6", 2100, json!({"exitCode":1,"message":msg}))),
            pod("x","oom","amd","containerStatuses",
                json!({"name":"c","image":"app:1","restartCount":3,"state":{"waiting":{"reason":"CrashLoopBackOff"}},
                       "lastState":{"terminated":{"exitCode":137,"reason":"OOMKilled"}}})),
        ]});
        let found = problems(&list, &nodes(), NOW);
        assert_eq!(found[0].pod, "oom", "healthy-node failures come first");
        assert_eq!(found[0].state, "OOMKilled");
        assert_eq!(found[1].cause.as_deref(), Some(
            "failed to start: Get \"https://192.0.2.1:6443/api/v1/namespaces/kube-system\": dial tcp 192.0.2.1:6443: i/o timeout"));
        assert_eq!(
            found[1].hint.as_deref(),
            Some("node down is NotReady — fix the node, not the pod")
        );
        assert!(!found[1].wants_log(&nodes()), "the kubelet is unreachable");
    }

    #[test]
    fn image_pull_backoff_uses_the_waiting_message() {
        let list = json!({"items":[pod("nv","dcgm","arm","containerStatuses",
            json!({"name":"exporter","image":"nvcr.io/nvidia/k8s/dcgm-exporter:4.5.3","restartCount":0,
                   "state":{"waiting":{"reason":"ImagePullBackOff","message":"Back-off pulling image \"nvcr.io/nvidia/k8s/dcgm-exporter:4.5.3\": not found"}}}))]});
        let found = problems(&list, &nodes(), NOW);
        assert_eq!(found[0].state, "ImagePullBackOff");
        assert_eq!(
            found[0].hint.as_deref(),
            Some("tag does not exist in the registry")
        );
    }

    #[test]
    fn old_restarts_on_a_running_container_are_history_not_a_problem() {
        let list = json!({"items":[pod("x","p","amd","containerStatuses",
            json!({"name":"c","image":"a:1","restartCount":2454,"state":{"running":{}},
                   "lastState":{"terminated":{"exitCode":137,"reason":"OOMKilled","finishedAt":"2026-01-01T00:00:00Z"}}}))]});
        assert!(problems(&list, &nodes(), NOW).is_empty());
    }

    #[test]
    fn cause_line_prefers_fatal_then_last_error_then_last_line() {
        assert_eq!(cause_line("a\nb\n").as_deref(), Some("b"));
        assert_eq!(
            cause_line("Traceback (most recent call last):\n  File \"x.py\", line 1\nValueError: bad\nshutting down").as_deref(),
            Some("ValueError: bad")
        );
        assert_eq!(
            cause_line("panic: boom\nerror: later").as_deref(),
            Some("panic: boom")
        );
        assert_eq!(cause_line("\n \n"), None);
    }

    #[test]
    fn a_moved_tag_shows_as_two_digests() {
        let st = |image: &str, digest: &str| json!({"name":"c","image":image,"imageID":format!("docker.io/x@{digest}")});
        let list = json!({"items":[
            pod("a","p1","amd","containerStatuses", st("furiosaai/furiosa-llm:latest","sha256:aaaa1111aaaa1111")),
            pod("a","p2","amd","containerStatuses", st("docker.io/furiosaai/furiosa-llm","sha256:bbbb2222bbbb2222")),
            pod("a","p3","amd","containerStatuses", st("furiosaai/furiosa-llm:2026.3.0","sha256:cccc")),
            pod("a","p4","amd","containerStatuses", st("ghcr.io/x/epp:main","sha256:dddd")),
        ]});
        let d = drift(&list);
        assert_eq!(
            d.len(),
            1,
            "pinned tags and single-digest floating tags are not drift"
        );
        assert_eq!(d[0].image, "furiosaai/furiosa-llm:latest");
        assert_eq!(
            d[0].line(),
            "furiosaai/furiosa-llm:latest runs 2 different builds (aaaa1111aaaa×1, bbbb2222bbbb×1) — the tag moved; pin a digest"
        );
    }
}
