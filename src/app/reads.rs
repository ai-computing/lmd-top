//! On-demand read-only previews; late responses must not replace newer UI state.
use super::App;

const LOADING: &str = "Loading resource YAML… (Esc to close)";
pub struct YamlRequest {
    pub generation: u64,
    pub kind: String,
    pub namespaced: bool,
    pub name: String,
}

impl App {
    pub fn perf_model_to_query(&self) -> Option<String> {
        if self.detail {
            self.perf_loading
                .clone()
                .or_else(|| self.perf_detail.as_ref().map(|detail| detail.model.clone()))
                .or_else(|| self.selected_perf_model())
        } else {
            self.selected_perf_model()
        }
    }

    pub fn yaml_is_loading(&self) -> bool {
        self.yaml_loading.is_some()
    }

    pub fn request_yaml(&mut self, kind: &str, namespaced: bool, name: String) {
        self.yaml_generation = self.yaml_generation.wrapping_add(1);
        let title = format!("{kind} {name} · yaml (read-only)");
        self.yaml_loading = Some((self.yaml_generation, title.clone()));
        self.yaml_request = Some(YamlRequest {
            generation: self.yaml_generation,
            kind: kind.into(),
            namespaced,
            name,
        });
        self.preview = Some((title, LOADING.into()));
        self.preview_scroll = 0;
        self.preview_apply = false;
    }

    pub fn invalidate_yaml_request(&mut self) {
        if let Some((_, title)) = &self.yaml_loading {
            if self.preview_apply
                || !self
                    .preview
                    .as_ref()
                    .is_some_and(|(t, body)| t == title && body == LOADING)
            {
                self.yaml_loading = None;
                self.yaml_request = None;
            }
        }
    }

    pub fn finish_yaml(&mut self, generation: u64, result: Result<String, String>) {
        self.invalidate_yaml_request();
        if self.yaml_loading.as_ref().map(|(id, _)| *id) != Some(generation) {
            return;
        }
        let (_, title) = self.yaml_loading.take().unwrap();
        let body = result.unwrap_or_else(|error| {
            format!("YAML request failed: {error}\nClose this preview and press y to retry.")
        });
        self.preview = Some((title, body));
        self.preview_scroll = 0;
        self.preview_apply = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn closed_reopened_and_replaced_previews_reject_old_results() {
        let mut app = App::new();
        app.request_yaml("pod", true, "first".into());
        let first = app.yaml_request.take().unwrap().generation;
        app.preview = None;
        app.finish_yaml(first, Ok("late".into()));
        assert!(app.preview.is_none());
        app.request_yaml("pod", true, "first".into());
        let second = app.yaml_request.take().unwrap().generation;
        app.finish_yaml(first, Err("old failure".into()));
        assert_eq!(app.preview.as_ref().unwrap().1, LOADING);
        app.finish_yaml(second, Ok("kind: Pod".into()));
        assert_eq!(app.preview.as_ref().unwrap().1, "kind: Pod");
        assert!(!app.preview_apply);
        app.request_yaml("node", false, "worker".into());
        let third = app.yaml_request.take().unwrap().generation;
        app.preview = Some(("Deployment".into(), "reviewed manifest".into()));
        app.preview_apply = true;
        app.finish_yaml(third, Ok("late node".into()));
        assert_eq!(app.preview.as_ref().unwrap().1, "reviewed manifest");
        assert!(app.preview_apply);
    }

    #[test]
    fn failed_read_remains_non_applicable_and_can_be_retried() {
        let mut app = App::new();
        app.request_yaml("job", true, "compile".into());
        let first = app.yaml_request.take().unwrap().generation;
        app.finish_yaml(first, Err("timeout".into()));
        assert!(app.preview.as_ref().unwrap().1.contains("timeout"));
        assert!(!app.preview_apply);
        app.request_yaml("job", true, "compile".into());
        assert_ne!(app.yaml_request.as_ref().unwrap().generation, first);
    }
}

#[cfg(test)]
mod performance_target_tests {
    use super::App;
    use crate::collect::{LatencyFamily, PerformanceTarget};
    #[test]
    fn prepared_detail_keeps_observed_service_and_family_when_snapshot_changes() {
        let mut app = App::new();
        app.snap.perf_targets.insert(
            "display-deployment".into(),
            PerformanceTarget {
                service: "actual-service".into(),
                families: [LatencyFamily::Furiosa].into(),
            },
        );
        app.request_perf_detail("display-deployment".into());
        app.snap.perf_targets.clear();
        let (_, model, target) = app.perf_request.take().unwrap();
        assert_eq!(model, "display-deployment");
        assert_eq!(target.service, "actual-service");
        assert_eq!(target.families, [LatencyFamily::Furiosa].into());
    }
}

#[cfg(test)]
mod detail_refresh_tests {
    use super::App;
    use crate::collect::PerfDetail;
    #[test]
    fn refresh_targets_open_detail_even_when_background_selection_disappears() {
        let mut app = App::new();
        app.detail = true;
        app.perf_detail = Some(PerfDetail {
            model: "opened-model".into(),
            ..PerfDetail::default()
        });
        assert_eq!(app.perf_model_to_query().as_deref(), Some("opened-model"));
        app.perf_loading = Some("pending-model".into());
        assert_eq!(app.perf_model_to_query().as_deref(), Some("pending-model"));
        app.detail = false;
        assert_eq!(app.perf_model_to_query(), None);
    }
}

#[cfg(test)]
mod route_review_tests {
    use super::*;
    #[test]
    fn route_forms_freeze_the_selected_snapshot_identity() {
        let mut app = App::new();
        app.view = crate::app::View::Routing;
        let first = crate::resource_guard::Identity {
            uid: "original".into(),
            version: "42".into(),
        };
        app.snap.routes.push(crate::collect::Route {
            configuration: None,
            match_summary: String::new(),
            backend_summary: String::new(),
            single_local_backend: true,
            identity: Some(first.clone()),
            route: "route".into(),
            path: "/a".into(),
            backend: "service".into(),
            kind: "Service".into(),
        });
        app.open_route_rename(app.selected_route().unwrap());
        app.snap.routes[0].identity.as_mut().unwrap().version = "43".into();
        assert_eq!(
            app.route_form.as_ref().unwrap().expected.as_ref(),
            Some(&first)
        );
        app.snap.service_names = vec!["actual-service".into()];
        app.open_route_retarget(app.selected_route().unwrap());
        assert_eq!(
            app.route_form.as_ref().unwrap().choices,
            vec!["Service:actual-service"]
        );
        assert_eq!(
            app.route_form
                .as_ref()
                .unwrap()
                .expected
                .as_ref()
                .unwrap()
                .version,
            "43"
        );
        app.snap.routes[0].identity = None;
        assert!(app.route_form.as_ref().unwrap().expected.is_some());
        app.route_form = None;
        app.snap.service_names.clear();
        app.open_route_retarget(app.selected_route().unwrap());
        assert!(app.route_form.is_none());
    }
}

#[cfg(test)]
mod route_navigation_tests {
    use super::*;
    #[test]
    fn pod_model_resolution_requires_unique_fresh_ownership() {
        let mut app = App::new();
        app.snap
            .deployment_pods
            .insert("deployment".into(), vec!["unrelated-pod".into()]);
        assert!(app.model_of_pod("unrelated-pod").is_none());
        crate::routing::fresh_relationship_sources(&mut app.snap);
        assert_eq!(
            app.model_of_pod("unrelated-pod").as_deref(),
            Some("deployment")
        );
        assert!(app.model_of_pod("deployment-lookalike").is_none());
        app.snap
            .deployment_pods
            .insert("other".into(), vec!["unrelated-pod".into()]);
        assert!(app.model_of_pod("unrelated-pod").is_none());
        app.snap.deployment_pods.remove("other");
        for source in ["pods", "replicasets", "workloads"] {
            crate::routing::fresh_relationship_sources(&mut app.snap);
            app.snap.sources.get_mut(source).unwrap().last_success_at = Some(1);
            assert!(app.model_of_pod("unrelated-pod").is_none(), "{source}");
        }
    }

    #[test]
    fn backend_names_do_not_replace_verified_deployment_relationships() {
        let mut app = App::new();
        app.view = crate::app::View::Routing;
        app.snap.routes.push(crate::collect::Route {
            configuration: None,
            match_summary: String::new(),
            backend_summary: String::new(),
            single_local_backend: true,
            identity: None,
            route: "route".into(),
            path: "/a".into(),
            backend: "public-service".into(),
            kind: "Service".into(),
        });
        assert!(app.selected_route_backend().is_none());
        app.pivot('m');
        assert!(matches!(app.view, crate::app::View::Routing));
        crate::routing::fresh_relationship_sources(&mut app.snap);
        app.snap
            .service_bindings
            .insert("real-deployment".into(), vec!["public-service".into()]);
        assert_eq!(
            app.selected_route_backend().as_deref(),
            Some("real-deployment")
        );
        app.snap
            .service_bindings
            .insert("other-deployment".into(), vec!["public-service".into()]);
        assert!(app.selected_route_backend().is_none());
        app.snap.service_bindings.remove("other-deployment");
        app.snap.routes[0].kind = "InferencePool".into();
        assert!(app.selected_route_backend().is_none());
        app.snap.pool_bindings.insert(
            "public-service".into(),
            vec!["first".into(), "second".into()],
        );
        assert!(app.selected_route_backend().is_none());
        app.snap
            .pool_bindings
            .insert("public-service".into(), vec!["real-deployment".into()]);
        assert_eq!(
            app.selected_route_backend().as_deref(),
            Some("real-deployment")
        );
        app.snap.pool_bindings.clear();
        app.pivot('m');
        assert!(matches!(app.view, crate::app::View::Routing));
    }
}
