//! Compare (Phase 8, decision D8): the selection, two at a time, and back
//! to browsing with the survivors selected.
use super::*;

impl SharprWindow {
    /// Compares `paths` (at least two) in this order.
    pub fn enter_compare_mode(&self, paths: Vec<PathBuf>) {
        {
            let state_rc = self.app_state();
            let mut state = state_rc.borrow_mut();
            if state.scope != ViewScope::Compare {
                state.previous_scope = Some(state.scope.clone());
            }
            state.scope = ViewScope::Compare;
            state.previous_content_page = Some("viewer".to_string());
        }
        if let Some(page) = self.imp().compare_page.borrow().as_ref() {
            page.start(paths);
        }
        // The side panels make way, which also keeps the filmstrip's file
        // actions out of reach.
        if self.imp().compare_panels.get().is_none() {
            self.imp().compare_panels.set(Some(self.hide_side_panels()));
        }
        if let Some(stack) = self.imp().content_stack.borrow().as_ref() {
            stack.set_transition_type(gtk4::StackTransitionType::SlideLeft);
            stack.set_visible_child_name("compare");
        }
        // Only now visible, the page can take the focus from the filmstrip.
        if let Some(page) = self.imp().compare_page.borrow().as_ref() {
            page.view().grab_focus();
        }
    }

    /// Back to browsing, with `survivors` selected and the first one shown.
    pub fn exit_compare_mode(&self, survivors: Vec<PathBuf>) {
        {
            let state_rc = self.app_state();
            let mut state = state_rc.borrow_mut();
            if state.scope == ViewScope::Compare {
                state.scope = state.previous_scope.take().unwrap_or_default();
            }
            state.previous_content_page = None;
            state.selected_paths = survivors
                .iter()
                .filter(|path| state.library.index_of_path(path).is_some())
                .cloned()
                .collect();
        }
        if let Some(stack) = self.imp().content_stack.borrow().as_ref() {
            stack.set_transition_type(gtk4::StackTransitionType::SlideRight);
            stack.set_visible_child_name("viewer");
        }
        if let Some(panels) = self.imp().compare_panels.take() {
            self.restore_side_panels(panels);
        }
        let first = survivors
            .first()
            .and_then(|path| self.imp().state.borrow().library.index_of_path(path));
        if let Some(filmstrip) = self.imp().filmstrip.borrow().as_ref() {
            if let Some(index) = first {
                filmstrip.navigate_to(index);
            }
            filmstrip.refresh_multi_selection_visuals();
        }
    }
}

/// Whether Compare is open.
pub(super) fn is_comparing(widget: &gtk4::Widget) -> bool {
    widget
        .downcast_ref::<SharprWindow>()
        .is_some_and(|win| win.app_state().borrow().scope == ViewScope::Compare)
}

/// Compare never changes files, and a crop works on one chosen image, so
/// the shortcuts and menus that change files are off during either (and
/// while a crop is being prepared).
pub(super) fn file_actions_blocked(widget: &gtk4::Widget) -> bool {
    is_comparing(widget)
        || widget.downcast_ref::<SharprWindow>().is_some_and(|win| {
            win.imp().crop_session.borrow().is_some() || win.imp().crop_preparing.get()
        })
}
