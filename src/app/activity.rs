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

/// One row in the unified Activity feed.
#[derive(Clone)]
pub struct ActivityRow {
    pub label: String,        // composed one-line summary
    pub pod: Option<String>,  // pod to tail logs from (Logs action)
    pub job: Option<String>,  // compile Job name to delete (Delete action); None for deploys
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
            let pct = match c.progress {
                Some(p) => format!(" {:.0}%", (p * 100.0).clamp(0.0, 100.0)),
                None => String::new(),
            };
            let vt = if c.target.is_empty() {
                c.vendor.clone()
            } else {
                format!("{} {}", c.vendor, c.target)
            };
            let running = c.status == "Running";
            out.push(ActivityRow {
                // running 은 진행바로 %를 그리므로 라벨엔 상태만.
                label: if running {
                    format!("compile · {} → {}   {}", c.model, vt, c.status)
                } else {
                    format!("compile · {} → {}   {}{}", c.model, vt, c.status, pct)
                },
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
                out.push(ActivityRow {
                    label: format!(
                        "deploy · {} ×{}   {} {} {}/{}",
                        m.name,
                        m.desired,
                        phase.glyph(),
                        phase.label(),
                        m.ready,
                        m.desired
                    ),
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
