//! Selection and contextual-header transitions for Survey.
use super::*;

pub(super) fn is_surveying(widget: &gtk4::Widget) -> bool {
    widget
        .downcast_ref::<SharprWindow>()
        .is_some_and(|window| window.app_state().borrow().scope == ViewScope::Survey)
}

impl SharprWindow {
    pub(super) fn setup_survey(&self, stack: &gtk4::Stack) {
        let page = crate::ui::survey::SurveyPage::new();
        let weak = self.downgrade();
        page.set_close_cb(move |survivors| {
            if let Some(window) = weak.upgrade() {
                window.exit_survey(survivors);
            }
        });
        stack.add_named(&page.root, Some("survey"));
        *self.imp().survey_page.borrow_mut() = Some(page.clone());
        let weak = self.downgrade();
        let was_visible = Cell::new(false);
        stack.connect_visible_child_name_notify(move |stack| {
            let visible = stack.visible_child_name().as_deref() == Some("survey");
            if was_visible.replace(visible) && !visible {
                let weak = weak.clone();
                let page = page.clone();
                glib::idle_add_local_once(move || {
                    let Some(window) = weak.upgrade() else { return };
                    let current = window
                        .imp()
                        .content_stack
                        .borrow()
                        .as_ref()
                        .and_then(|stack| stack.visible_child_name());
                    if current.as_deref() == Some("survey") {
                        return;
                    }
                    if is_surveying(window.upcast_ref()) {
                        page.close();
                    } else {
                        // A completed copy can navigate after the user opened
                        // Survey. Retire its worker without replacing that view.
                        page.clear();
                        if !file_actions_blocked(window.upcast_ref()) {
                            window.set_mode_header(None);
                            if let Some(panels) = window.imp().review_panels.take() {
                                window.restore_side_panels(panels);
                            }
                        }
                    }
                });
            }
        });
        let action = gio::SimpleAction::new("survey", None);
        action.set_enabled(false);
        let weak = self.downgrade();
        action.connect_activate(move |_, _| {
            let Some(window) = weak.upgrade() else { return };
            if file_actions_blocked(window.upcast_ref()) || window.visible_dialog().is_some() {
                return;
            }
            let paths = {
                let state = window.imp().state.borrow();
                ordered_selection(
                    (0..state.library.image_count())
                        .filter_map(|i| state.library.entry_at(i))
                        .map(|e| e.path()),
                    &state.selected_paths,
                )
            };
            if let Some(paths) = paths {
                window.enter_survey(paths);
            }
        });
        self.add_action(&action);
    }

    pub(crate) fn enter_survey(&self, paths: Vec<PathBuf>) {
        if paths.len() < 2 || file_actions_blocked(self.upcast_ref()) {
            return;
        }
        {
            let state = self.app_state();
            let mut state = state.borrow_mut();
            state.previous_scope = Some(state.scope.clone());
            state.scope = ViewScope::Survey;
            state.previous_content_page = Some("viewer".into());
        }
        if let Some(page) = self.imp().survey_page.borrow().as_ref() {
            page.start(paths);
            self.set_mode_header(Some(&page.header));
        }
        if self.imp().review_panels.get().is_none() {
            self.imp().review_panels.set(Some(self.hide_side_panels()));
        }
        if let Some(stack) = self.imp().content_stack.borrow().as_ref() {
            stack.set_visible_child_name("survey");
        }
        if let Some(page) = self.imp().survey_page.borrow().as_ref() {
            page.grid.grab_focus();
        }
    }

    fn exit_survey(&self, survivors: Vec<PathBuf>) {
        self.set_mode_header(None);
        {
            let state = self.app_state();
            let mut state = state.borrow_mut();
            state.scope = state.previous_scope.take().unwrap_or_default();
            state.previous_content_page = None;
            state.selected_paths = survivors
                .iter()
                .filter(|path| state.library.index_of_path(path).is_some())
                .cloned()
                .collect();
            state.selection_explicit = true;
        }
        if let Some(stack) = self.imp().content_stack.borrow().as_ref() {
            stack.set_visible_child_name("viewer");
        }
        if let Some(panels) = self.imp().review_panels.take() {
            self.restore_side_panels(panels);
        }
        if let Some(strip) = self.imp().filmstrip.borrow().as_ref() {
            if let Some(first) = survivors
                .first()
                .and_then(|path| self.imp().state.borrow().library.index_of_path(path))
            {
                strip.navigate_to(first);
            }
            strip.refresh_multi_selection_visuals();
        }
    }
}
