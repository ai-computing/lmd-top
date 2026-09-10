//! Headless and CLI planning methods for compile and deploy actions.

use crate::app::{App, Pending, View};
use crate::collect::ModelArtifact;

use super::fields::{apply_field_overrides, override_value};

/// Create a synthetic model artifact for headless planning.
pub fn synthetic_artifact_for(
    model_id: &str,
    vendor: &'static str,
    mount: String,
    overrides: &[(String, String)],
) -> ModelArtifact {
    let model_name = model_id.rsplit('/').next().unwrap_or(model_id).to_string();
    let pack = crate::accel::by_id(vendor);
    let engine = pack.map(|p| p.engine).unwrap_or("vLLM");
    // Default tensor-parallel width is the accelerator's declared maximum group size: a CA22
    // exposes 4 chips, one RNGD 8 PEs, a single GPU 1.
    let tp_default = pack
        .and_then(|p| p.caps.max_tensor_parallel)
        .unwrap_or(1)
        .to_string();
    let mut opts = vec![(
        "tp".into(),
        override_value(overrides, "tp").unwrap_or(tp_default),
    )];
    if let Some(v) = override_value(overrides, "max-len") {
        opts.push(("max-len".into(), v));
    }
    ModelArtifact {
        model: model_name.clone(),
        family: model_name.to_lowercase(),
        engine: engine.into(),
        node: String::new(),
        image: String::new(),
        source: model_id.into(),
        mount,
        // Planned, not observed — there is no node-local directory yet.
        host_path: None,
        opts,
    }
}

/// Extract the pending apply plan (title and YAML), or return the descriptive error.
pub fn take_apply_plan(app: &mut App) -> Result<(String, String), String> {
    match app.confirm.take() {
        Some(Pending::Apply { title, yaml }) => {
            app.confirm_yes = false;
            Ok((title, yaml))
        }
        _ => {
            // Forward rejection reasons left in preview or toast to prevent silent headless failures
            if let Some((title, body)) = app.preview.take() {
                return Err(format!(
                    "{}\n{}",
                    title,
                    body.lines()
                        .filter(|l| !l.trim().is_empty())
                        .take(6)
                        .collect::<Vec<_>>()
                        .join("\n")
                ));
            }
            if let Some(t) = app.toast.take() {
                return Err(t);
            }
            Err("operation did not produce an apply manifest".into())
        }
    }
}

/// Plan a compilation job for a model in headless mode and return (title, yaml).
pub fn plan_compile_for_model(
    app: &mut App,
    model_id: &str,
    vendor: &'static str,
    overrides: &[(String, String)],
) -> Result<(String, String), String> {
    let a = synthetic_artifact_for(model_id, vendor, String::new(), overrides);
    let mut form = app.build_compile_form(&a, vendor);
    apply_field_overrides(&mut form.fields, overrides);
    app.compile_form = Some(form);
    app.compile_form_submit();
    take_apply_plan(app)
}

/// Plan a serving deployment for a model in headless mode and return (title, yaml).
pub fn plan_deploy_for_model(
    app: &mut App,
    model_id: &str,
    vendor: &'static str,
    overrides: &[(String, String)],
) -> Result<(String, String), String> {
    let repo_dir = model_id.replace('/', "--");
    let default_mount = match vendor {
        "rbln" => {
            let target = {
                let a = synthetic_artifact_for(model_id, vendor, String::new(), overrides);
                let mut form = app.build_compile_form(&a, vendor);
                apply_field_overrides(&mut form.fields, overrides);
                form.target()
            };
            format!("/mnt/store/compiled/{}/{}/{}", repo_dir, vendor, target)
        }
        "furiosa" => {
            override_value(overrides, "mount").unwrap_or_else(|| model_id.to_string())
        }
        _ => model_id.to_string(),
    };
    let mount = override_value(overrides, "mount").unwrap_or(default_mount);
    let old_view = app.view;
    let old_focus = app.panel_focus;
    let old_selected = app.selected;
    let old_len = app.snap.artifacts.len();
    let a = synthetic_artifact_for(model_id, vendor, mount, overrides);
    app.snap.artifacts.push(a);
    app.view = View::Serving;
    app.panel_focus = 0;
    // `open_deploy_form` reads the selection through `sel_orig()`, i.e. `order()[selected]` —
    // a *display-order* lookup. Assigning the raw push index here made the Serving view's
    // grouping resolve to some unrelated cluster artifact, so `--plan deploy --model X`
    // silently emitted a Deployment for whatever model happened to sort into that slot.
    // Translate the raw index into its display position instead.
    app.selected = app
        .order()
        .iter()
        .position(|&i| i == old_len)
        .unwrap_or(old_len);
    app.open_deploy_form();
    app.snap.artifacts.truncate(old_len);
    app.view = old_view;
    app.panel_focus = old_focus;
    app.selected = old_selected;
    let Some(form) = app.deploy_form.as_mut() else {
        return Err("failed to create deploy form".into());
    };
    // Defence in depth: a headless planner must never plan a different model than it was asked
    // for. If the form did not land on the requested artifact, fail loudly rather than emit YAML.
    if form.model_id != model_id && form.model != model_id {
        let got = form.model_id.clone();
        app.deploy_form = None;
        return Err(format!(
            "deploy plan resolved to '{}' instead of '{}' — refusing to emit a manifest for the wrong model",
            got, model_id
        ));
    }
    apply_field_overrides(&mut form.fields, overrides);
    app.deploy_form_submit();
    take_apply_plan(app)
}
