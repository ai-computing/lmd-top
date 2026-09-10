//! Shared-store maintenance from the Deploy▸Library tree — remove or relocate a build.
//!
//! Both operations run as Jobs (see [`crate::store`]), so they take the same route as compile
//! and deploy: build the manifest, show it in the preview, and let the operator apply it. That
//! keeps `rm -rf` reviewable before it runs, and puts it in the audit log afterwards.

use crate::app::App;
use crate::ops::StoreForm;

impl App {
    /// The PVC backing the shared store, as the discovery CronJob names it.
    fn store_pvc(&self) -> String {
        "model-store".to_string()
    }

    /// Build a delete manifest for the selected store row and show it in the preview.
    ///
    /// Refuses outright when a Deployment appears to be serving out of the path — reclaiming
    /// space is never worth breaking a live endpoint, and the operator can stop the deployment
    /// first if they meant it.
    pub fn open_store_delete(&mut self) {
        let Some(sel) = self.selected_stored().cloned() else {
            return;
        };
        let path = match crate::store::validate_path(&sel.path) {
            Ok(p) => p,
            Err(e) => {
                self.notify(format!("store: {} ({})", e, sel.path));
                return;
            }
        };
        if crate::store::entry_for(&path, &self.snap.stored).is_none() {
            self.notify("store: that path is not in the current inventory".into());
            return;
        }
        let users = crate::store::in_use_by(&path, &self.snap.artifacts);
        if !users.is_empty() {
            self.notify(format!(
                "store: {} is serving from this build — stop it first",
                users.join(", ")
            ));
            return;
        }
        let m = crate::store::delete_manifest(&self.ns, &self.store_pvc(), &path, &sel.size);
        self.preview = Some((
            format!("delete {} [{}] · {} — review, then a", sel.repo, sel.compiled_for, sel.size),
            m.to_yaml(),
        ));
        self.preview_scroll = 0;
        self.preview_apply = true;
    }

    /// Open the move form with the current path as the starting value.
    pub fn open_store_move(&mut self) {
        let Some(sel) = self.selected_stored().cloned() else {
            return;
        };
        match crate::store::validate_path(&sel.path) {
            Ok(p) => {
                self.store_form = Some(StoreForm {
                    src: p.clone(),
                    repo: sel.repo,
                    size: sel.size,
                    value: p,
                });
            }
            Err(e) => self.notify(format!("store: {} ({})", e, sel.path)),
        }
    }

    /// Validate the typed destination and turn it into a preview-able move manifest.
    pub fn submit_store_move(&mut self) {
        let Some(f) = self.store_form.clone() else {
            return;
        };
        let dest = match crate::store::validate_dest(&f.value, &f.src, &self.snap.stored) {
            Ok(d) => d,
            Err(e) => {
                self.notify(format!("store: {}", e));
                return;
            }
        };
        let users = crate::store::in_use_by(&f.src, &self.snap.artifacts);
        if !users.is_empty() {
            self.notify(format!(
                "store: {} is serving from this build — moving it would break the mount",
                users.join(", ")
            ));
            return;
        }
        let m = crate::store::move_manifest(&self.ns, &self.store_pvc(), &f.src, &dest);
        self.store_form = None;
        self.preview = Some((
            format!("move {} → {} — review, then a", f.src, dest),
            m.to_yaml(),
        ));
        self.preview_scroll = 0;
        self.preview_apply = true;
    }
}

impl App {
    /// The selected artifact's (deployment, node, hostPath), when a provenance probe is possible.
    ///
    /// Requires a node-local directory: a PVC-backed build already has its toolchain in the
    /// store inventory, and there is nothing to probe for an `emptyDir`.
    pub fn provenance_target(&mut self) -> Option<(String, String, String)> {
        let a = self.selected_artifact()?;
        let (deployment, node, raw) = (a.model.clone(), a.node.clone(), a.host_path.clone());
        if node.is_empty() {
            self.notify("provenance: this deployment has no scheduled node".into());
            return None;
        }
        let Some(raw) = raw else {
            self.notify(
                "provenance: not a node-local artifact — store builds show it in Deploy▸Library"
                    .into(),
            );
            return None;
        };
        match crate::probe::validate_host_path(&raw) {
            Ok(p) => Some((deployment, node, p)),
            Err(e) => {
                self.notify(format!("provenance: {}", e));
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::app::App;
    use crate::collect::{ModelArtifact, StoredModel};

    fn stored(repo: &str, path: &str) -> StoredModel {
        StoredModel {
            repo: repo.into(),
            family: repo.into(),
            revision: "-".into(),
            format: "rbln".into(),
            compiled_for: "RBLN-CA22-tp4".into(),
            size: "12G".into(),
            path: path.into(),
            built_with: "optimum-rbln=0.10.2".into(),
        }
    }

    /// An App with one store row selected in the Library tree.
    fn app_with_store() -> App {
        let mut a = App::new();
        a.snap.stored = vec![stored("org/name", "compiled/org--name/rbln/tp4")];
        a.goto_view(crate::app::View::Library);
        a.panel_focus = 0;
        // Select the stored row in the unified tree.
        for _ in 0..40 {
            if matches!(a.selected_lib_item(), Some(crate::app::LibItem::Stored(_))) {
                break;
            }
            a.move_sel(1);
        }
        a
    }

    /// Provenance is only worth collecting if an operator can see it. Render the tree and the
    /// detail panel and check the toolchain is on screen in both.
    #[test]
    fn the_toolchain_that_built_a_store_row_is_visible() {
        use ratatui::backend::TestBackend;
        use ratatui::Terminal;
        let _g = crate::ui::RENDER_TEST_LOCK
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let mut a = app_with_store();
        let screen = |a: &App| {
            let mut fx = crate::ui::FxState::disabled();
            let mut t = Terminal::new(TestBackend::new(160, 40)).unwrap();
            t.draw(|f| crate::ui::draw(f, a, &mut fx)).unwrap();
            let buf = t.backend().buffer().clone();
            let mut s = String::new();
            for y in 0..buf.area.height {
                for x in 0..buf.area.width {
                    if let Some(c) = buf.cell((x, y)) {
                        s.push_str(c.symbol());
                    }
                }
                s.push('\n');
            }
            s
        };
        assert!(
            screen(&a).contains("optimum-rbln=0.10.2"),
            "the tree row should carry the toolchain"
        );
        a.detail = true;
        let d = screen(&a);
        assert!(d.contains("built-with"), "detail panel needs a built-with row");
        assert!(d.contains("optimum-rbln=0.10.2"), "with the value");
    }

    /// Free space belongs on the view where builds are created and removed. Render it.
    #[test]
    fn store_free_space_is_shown_on_the_library_view() {
        use ratatui::backend::TestBackend;
        use ratatui::Terminal;
        let _g = crate::ui::RENDER_TEST_LOCK
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let mut a = app_with_store();
        a.snap.store_capacity = Some(crate::collect::StoreCapacity {
            total_bytes: 57_504_801_730_560,
            used_bytes: 7_038_588_125_184,
            avail_bytes: 50_466_213_605_376,
        });
        let render = |a: &App| {
            let mut fx = crate::ui::FxState::disabled();
            let mut t = Terminal::new(TestBackend::new(180, 40)).unwrap();
            t.draw(|f| crate::ui::draw(f, a, &mut fx)).unwrap();
            let buf = t.backend().buffer().clone();
            let mut s = String::new();
            for y in 0..buf.area.height {
                for x in 0..buf.area.width {
                    if let Some(c) = buf.cell((x, y)) {
                        s.push_str(c.symbol());
                    }
                }
                s.push('\n');
            }
            s
        };
        let screen = render(&a);
        assert!(screen.contains("46T free"), "free space missing from Library");
        assert!(screen.contains("52T"), "total missing");
        assert!(screen.contains("12% used"), "used share missing");

        // An unmeasured store must simply omit the figures, not print zeros.
        a.snap.store_capacity = None;
        let bare = render(&a);
        assert!(!bare.contains("free"), "must not invent a figure when unmeasured");
    }

    /// A build whose provenance the scan could not read must say so, not show a blank.
    #[test]
    fn unknown_provenance_is_labelled_not_blank() {
        use ratatui::backend::TestBackend;
        use ratatui::Terminal;
        let _g = crate::ui::RENDER_TEST_LOCK
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let mut a = app_with_store();
        a.snap.stored[0].built_with = "-".into();
        a.detail = true;
        let mut fx = crate::ui::FxState::disabled();
        let mut t = Terminal::new(TestBackend::new(160, 40)).unwrap();
        t.draw(|f| crate::ui::draw(f, &a, &mut fx)).unwrap();
        let buf = t.backend().buffer().clone();
        let mut s = String::new();
        for y in 0..buf.area.height {
            for x in 0..buf.area.width {
                if let Some(c) = buf.cell((x, y)) {
                    s.push_str(c.symbol());
                }
            }
        }
        assert!(s.contains("unknown"), "must say unknown: {}", &s[..s.len().min(400)]);
    }

    #[test]
    fn delete_puts_a_reviewable_manifest_in_the_preview() {
        let mut a = app_with_store();
        assert!(
            matches!(a.selected_lib_item(), Some(crate::app::LibItem::Stored(_))),
            "test needs a stored row selected"
        );
        a.open_store_delete();
        let (title, yaml) = a.preview.clone().expect("a preview");
        assert!(title.contains("delete"), "{}", title);
        assert!(yaml.contains("kind: Job"), "{}", yaml);
        assert!(yaml.contains("/mnt/store/compiled/org--name/rbln/tp4"), "{}", yaml);
        // Applying is the operator's next keystroke, not something that already happened.
        assert!(a.preview_apply);
    }

    #[test]
    fn delete_refuses_while_a_deployment_serves_from_the_path() {
        let mut a = app_with_store();
        a.snap.artifacts = vec![ModelArtifact {
            model: "vllm-rbln-x".into(),
            family: "name".into(),
            engine: "vllm".into(),
            node: "n".into(),
            image: "i".into(),
            source: "/mnt/store/compiled/org--name/rbln/tp4".into(),
            mount: "/mnt/store ← pvc/model-store".into(),
            host_path: None,
            opts: vec![],
        }];
        a.open_store_delete();
        assert!(a.preview.is_none(), "must not offer to delete a live build");
        assert!(
            a.toast.clone().unwrap_or_default().contains("vllm-rbln-x"),
            "the message must name what is using it: {:?}",
            a.toast
        );
    }

    #[test]
    fn a_store_row_offers_rescan_alongside_the_destructive_actions() {
        let mut a = app_with_store();
        a.open_action_menu();
        let menu = a.action_menu.as_ref().expect("an action menu on a store row");
        let keys: Vec<char> = menu.items.iter().map(|i| i.key).collect();
        for want in ['V', 'M', 'r'] {
            assert!(keys.contains(&want), "store row menu is missing {:?}: {:?}", want, keys);
        }
        // Removal is irreversible; a rescan only reads. They must not share a tier.
        let mode_of = |k: char| {
            menu.items
                .iter()
                .find(|i| i.key == k)
                .map(|i| i.action.required_mode())
                .unwrap()
        };
        assert_eq!(mode_of('M'), crate::app::Mode::Danger);
        assert_eq!(mode_of('r'), crate::app::Mode::Admin);
        assert_eq!(mode_of('V'), crate::app::Mode::Admin);
    }

    #[test]
    fn move_rejects_a_destination_that_escapes_the_store() {
        let mut a = app_with_store();
        a.open_store_move();
        a.store_form.as_mut().unwrap().value = "../../etc/cron.d".into();
        a.submit_store_move();
        assert!(a.preview.is_none(), "traversal must not reach a manifest");
        assert!(a.store_form.is_some(), "form stays open so it can be fixed");
    }

    #[test]
    fn move_builds_a_manifest_for_a_good_destination() {
        let mut a = app_with_store();
        a.open_store_move();
        assert_eq!(
            a.store_form.as_ref().unwrap().value,
            "compiled/org--name/rbln/tp4",
            "the form starts at the current path"
        );
        a.store_form.as_mut().unwrap().value = "compiled/archive/org--name/rbln/tp4".into();
        a.submit_store_move();
        let (title, yaml) = a.preview.clone().expect("a preview");
        assert!(title.contains("move"), "{}", title);
        assert!(yaml.contains("compiled/archive/org--name/rbln/tp4"), "{}", yaml);
        assert!(a.store_form.is_none(), "form closes once the manifest is built");
    }
}
