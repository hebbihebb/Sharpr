use super::*;

impl SharprWindow {
    pub fn enter_compare_mode(&self, item: CompareItem) {
        {
            let state_rc = self.app_state();
            let mut state = state_rc.borrow_mut();
            if state.scope != ViewScope::Compare {
                state.previous_scope = Some(state.scope.clone());
            }
            state.scope = ViewScope::Compare;
            state.previous_content_page = Some("viewer".to_string());
            state.comparison = Some(item.clone());
        }
        if let Some(page) = self.imp().compare_page.borrow().as_ref() {
            page.set_comparison(item);
        }
        if let Some(stack) = self.imp().content_stack.borrow().as_ref() {
            stack.set_transition_type(gtk4::StackTransitionType::SlideLeft);
            stack.set_visible_child_name("compare");
        }
    }

    pub fn exit_compare_mode(&self) {
        {
            let state_rc = self.app_state();
            let mut state = state_rc.borrow_mut();
            if state.scope == ViewScope::Compare {
                state.scope = state.previous_scope.take().unwrap_or_default();
            }
            state.previous_content_page = None;
            state.comparison = None;
        }
        if let Some(page) = self.imp().compare_page.borrow().as_ref() {
            page.clear();
        }
        if let Some(stack) = self.imp().content_stack.borrow().as_ref() {
            stack.set_transition_type(gtk4::StackTransitionType::SlideRight);
            stack.set_visible_child_name("viewer");
        }
    }
}
