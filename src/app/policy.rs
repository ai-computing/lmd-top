//! Routing policy workspace entry and asynchronous, read-only rollout preview.
use super::*;
impl App {
    pub fn open_policy_form(&mut self) {
        if self.policy_rx.is_some() {
            self.notify("policy preview is still loading".into());
            return;
        }
        self.policy_draft = None;
        self.policy_form = Some(CompileForm {
            model: "Routing policy".into(),
            model_id: String::new(),
            vendor: "gpu",
            engine: String::new(),
            dest: String::new(),
            cursor: 0,
            editing: false,
            fields: vec![CompileField {
                key: "file".into(),
                label: "Policy file".into(),
                value: std::env::var("LMD_POLICY").unwrap_or_else(|_| "policy.yaml".into()),
                choices: vec![],
                numeric: false,
                help: "lmd-top --policy init --workspace ./my-policy creates a tested starter"
                    .into(),
            }],
        });
    }
    pub fn policy_form_submit(&mut self) {
        let Some(form) = &self.policy_form else {
            return;
        };
        let mut policy = if let Some(draft) = &self.policy_draft {
            draft.clone()
        } else {
            let policy = match crate::policy::Policy::read(&form.get("file")) {
                Ok(p) => p,
                Err(e) => {
                    self.notify(format!("policy: {e}"));
                    return;
                }
            };
            let field =
                |key: &str, label: &str, value: String, numeric: bool, help: &str| CompileField {
                    key: key.into(),
                    label: label.into(),
                    value,
                    numeric,
                    choices: vec![],
                    help: help.into(),
                };
            let mut fields = vec![
                field(
                    "image",
                    "EPP image",
                    policy.image.clone(),
                    false,
                    "Use the tested plugin image tag/digest",
                ),
                field(
                    "profile",
                    "Profile",
                    policy.profile.clone(),
                    false,
                    "Existing scheduling profile to extend",
                ),
                field(
                    "weight",
                    "Weight",
                    policy.weight.to_string(),
                    true,
                    "Contribution alongside existing scorers; not a traffic percentage",
                ),
                field(
                    "age",
                    "Metric age (s)",
                    policy.parameters.max_metric_age_seconds.to_string(),
                    true,
                    "Reject older load samples",
                ),
            ];
            for (i, c) in policy.parameters.classes.iter().enumerate() {
                fields.push(field(
                    &format!("capacity-{i}"),
                    &format!("{} req/s", c.name),
                    c.capacity.to_string(),
                    true,
                    "Measured capacity for equivalent model and workload",
                ));
            }
            let form = self.policy_form.as_mut().unwrap();
            form.fields = fields;
            form.cursor = 0;
            form.model = format!("{} / {}", policy.deployment, policy.config_map);
            self.policy_draft = Some(policy);
            return;
        };
        let form = self.policy_form.as_ref().unwrap();
        policy.image = form.get("image");
        policy.profile = form.get("profile");
        let parsed = (|| -> Result<(), String> {
            policy.weight = form
                .get("weight")
                .parse()
                .map_err(|_| "weight must be numeric".to_string())?;
            policy.parameters.max_metric_age_seconds = form
                .get("age")
                .parse()
                .map_err(|_| "metric age must be numeric".to_string())?;
            for (i, c) in policy.parameters.classes.iter_mut().enumerate() {
                c.capacity = form
                    .get(&format!("capacity-{i}"))
                    .parse()
                    .map_err(|_| "capacity must be numeric".to_string())?;
            }
            policy.validate().map_err(|e| e.to_string())
        })();
        if let Err(e) = parsed {
            self.notify(format!("policy: {e}"));
            return;
        }
        let (tx, rx) = std::sync::mpsc::channel();
        self.policy_rx = Some(rx);
        let ns = self.ns.clone();
        tokio::spawn(async move {
            let _ = tx.send(policy.plan(&ns).await.map_err(|e| e.to_string()));
        });
        self.policy_form = None;
        self.notify("reading EPP deployment/config for policy preview…".into());
    }
    pub fn poll_policy(&mut self) {
        let Some(rx) = &self.policy_rx else { return };
        match rx.try_recv() {
            Ok(result) => {
                self.policy_rx = None;
                match result {
                    Ok(yaml) => {
                        self.preview = Some((
                            "Routing policy · new config + EPP image · review before apply".into(),
                            yaml,
                        ));
                        self.preview_scroll = 0;
                        self.preview_apply = true;
                        self.preview_required = crate::app::Mode::Admin;
                    }
                    Err(e) => {
                        self.preview = Some(("Routing policy — preview failed".into(), e));
                        self.preview_apply = false;
                        self.preview_scroll = 0;
                    }
                }
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                self.policy_rx = None;
                self.notify("policy preview worker stopped".into());
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
        }
    }
}
