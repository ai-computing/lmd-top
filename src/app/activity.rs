//! Unified activity — compile Jobs and deploy rollouts as one operations feed,
//! plus per-deployment lifecycle phase (Serving/Starting/Degraded/Failed) derived
//! by cross-referencing pods. Shared by the Serving view and the Activity view.

use super::*;

/// A running/serving deployment's lifecycle phase, inferred from replicas + pods.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DeployPhase {
    Serving,   // ready == desired, no failing pods
    Starting,  // desired > 0, ready < desired, pods scheduling/pulling
    Degraded,  // some ready, but a pod is crashing/failed
    Failed,    // desired > 0, ready == 0, pods crashing/failed
    ScaledZero, // desired == 0
}

impl DeployPhase {
    pub fn label(self) -> &'static str {
        match self {
            DeployPhase::Serving => "Serving",
            DeployPhase::Starting => "Starting",
            DeployPhase::Degraded => "Degraded",
            DeployPhase::Failed => "Failed",
            DeployPhase::ScaledZero => "Scaled-0",
        }
    }
    /// Attention rank for sorting (higher = needs attention first).
    pub fn rank(self) -> u8 {
        match self {
            DeployPhase::Failed => 4,
            DeployPhase::Degraded => 3,
            DeployPhase::Starting => 2,
            DeployPhase::Serving => 1,
            DeployPhase::ScaledZero => 0,
        }
    }
    /// Severity glyph for the phase (legible in the colorblind theme).
    pub fn glyph(self) -> &'static str {
        match self {
            DeployPhase::Serving => "●",
            DeployPhase::Starting => "◐",
            DeployPhase::Degraded => "⚠",
            DeployPhase::Failed => "✗",
            DeployPhase::ScaledZero => "○",
        }
    }
}

/// One row in the unified Activity feed. Structured so the view can lay it out
/// as aligned columns (kind · target · status), matching the other list views.
#[derive(Clone)]
pub struct ActivityRow {
    pub kind: &'static str,    // "compile" | "deploy"
    pub target: String,        // "model → vendor target" / "model ×N"
    pub status: String,        // "Running" / "Starting 0/2" / "Failed" / "Complete"
    pub sev: u8,               // 0 ok · 1 warn/active · 2 bad  (view maps to color)
    pub label: String,         // searchable one-line (kind + target + status)
    pub pod: Option<String>,   // pod to tail logs from (Logs action)
    pub job: Option<String>,   // compile Job name to delete (Delete action); None for deploys
    pub running_compile: bool, // a compile Job still in progress → show a progress bar
    pub progress: Option<f32>, // compile progress 0.0..1.0 (None = indeterminate)
}

impl App {
    /// Lifecycle phase of a deployment — "is it serving, still trying, or failed?".
    /// Cross-references its pods (phase Failed / high restart count = crashing).
    pub fn deploy_phase(&self, model: &str, desired: i64, ready: i64) -> DeployPhase {
        if desired == 0 {
            return DeployPhase::ScaledZero;
        }
        let pods: Vec<&crate::collect::PodRow> = self
            .snap
            .pods
            .iter()
            .filter(|p| p.name.starts_with(model))
            .collect();
        // CrashLoopBackOff shows up as a Failed phase or a rapidly-restarting pod.
        let failing = pods
            .iter()
            .any(|p| p.phase == "Failed" || p.restarts >= 3);
        if failing {
            return if ready > 0 {
                DeployPhase::Degraded
            } else {
                DeployPhase::Failed
            };
        }
        if ready >= desired {
            DeployPhase::Serving
        } else {
            DeployPhase::Starting
        }
    }

    fn pod_for(&self, prefix: &str) -> Option<String> {
        self.snap
            .pods
            .iter()
            .find(|p| p.name.starts_with(prefix))
            .map(|p| p.name.clone())
    }

    /// Unified operations feed: in-flight/recent compile Jobs first, then deploy
    /// rollouts that are still trying or have failed (steady "Serving" ones are
    /// calm and live in the Serving view, so they are omitted here).
    pub fn activity_rows(&self) -> Vec<ActivityRow> {
        let mut out: Vec<ActivityRow> = Vec::new();

        // Compile Jobs — always shown (Running/Pending/Complete/Failed).
        for c in &self.snap.compiles {
            let vt = if c.target.is_empty() {
                c.vendor.clone()
            } else {
                format!("{} {}", c.vendor, c.target)
            };
            let running = c.status == "Running";
            let pct = match c.progress {
                Some(p) => format!(" {:.0}%", (p * 100.0).clamp(0.0, 100.0)),
                None => String::new(),
            };
            let target = format!("{} → {}", c.model, vt);
            // 상태 텍스트에 진행률(%)을 항상 함께 — 진행바와 별개로 숫자로도 보이게.
            let status = format!("{}{}", c.status, pct);
            let sev = match c.status.as_str() {
                "Failed" => 2,
                "Complete" => 0,
                _ => 1,
            };
            out.push(ActivityRow {
                kind: "compile",
                label: format!("compile {} {}", target, status),
                target,
                status,
                sev,
                pod: self.pod_for(&c.name),
                job: Some(c.name.clone()),
                running_compile: running,
                progress: c.progress,
            });
        }

        // Deploy rollouts — only the not-yet-steady ones (trying / failed).
        for m in &self.snap.models {
            let phase = self.deploy_phase(&m.name, m.desired, m.ready);
            if matches!(
                phase,
                DeployPhase::Starting | DeployPhase::Degraded | DeployPhase::Failed
            ) {
                let target = format!("{} ×{}", m.name, m.desired);
                let status = format!("{} {} {}/{}", phase.glyph(), phase.label(), m.ready, m.desired);
                let sev = match phase {
                    DeployPhase::Failed => 2,
                    _ => 1,
                };
                out.push(ActivityRow {
                    kind: "deploy",
                    label: format!("deploy {} {}", target, status),
                    target,
                    status,
                    sev,
                    pod: self.pod_for(&m.name),
                    job: None,
                    running_compile: false,
                    progress: None,
                });
            }
        }
        out
    }

    /// The Activity row under the cursor (Activity view uses identity ordering).
    pub fn selected_activity(&self) -> Option<ActivityRow> {
        let i = self.sel_orig()?;
        self.activity_rows().into_iter().nth(i)
    }
}
