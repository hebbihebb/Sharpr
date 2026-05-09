use super::*;

impl SharprWindow {
    pub fn enter_compare_mode(&self, item: CompareItem) {
        let state_rc = self.app_state();
        {
            let mut state = state_rc.borrow_mut();
            if state.scope != ViewScope::Compare {
                state.previous_scope = Some(state.scope.clone());
                state.scope = ViewScope::Compare;
            }
            state.previous_content_page = self
                .imp()
                .content_stack
                .borrow()
                .as_ref()
                .and_then(|stack| stack.visible_child_name())
                .filter(|name| name.as_str() != "compare")
                .map(|name| name.to_string());

            if !state
                .compare_queue
                .iter()
                .any(|i| i.output_path == item.output_path)
            {
                state.compare_queue.push(item.clone());
            }
            state.selected_compare_output = Some(item.output_path.clone());
        }

        self.refresh_compare_view();
    }

    pub fn exit_compare_mode(&self) {
        self.exit_compare_mode_internal(false);
    }

    fn exit_compare_mode_internal(&self, restore_page: bool) {
        let state_rc = self.app_state();
        let (should_restore, prev_scope, prev_page) = {
            let mut state = state_rc.borrow_mut();
            if state.scope == ViewScope::Compare {
                if let Some(prev) = state.previous_scope.take() {
                    state.scope = prev.clone();
                    let prev_page = if restore_page {
                        state.previous_content_page.take()
                    } else {
                        None
                    };
                    (true, Some(prev), prev_page)
                } else {
                    (false, None, None)
                }
            } else {
                (false, None, None)
            }
        };

        if should_restore {
            if let Some(prev) = prev_scope {
                self.restore_view_for_scope(prev);
            }
            if let Some(prev_page) = prev_page {
                if let Some(stack) = self.imp().content_stack.borrow().as_ref() {
                    stack.set_transition_type(gtk4::StackTransitionType::SlideRight);
                    stack.set_visible_child_name(&prev_page);
                }
            }
        }
    }

    pub fn refresh_compare_view(&self) {
        let (items, selected_output) = {
            let state = self.app_state();
            let mut state = state.borrow_mut();
            if state.selected_compare_output.is_none() && !state.compare_queue.is_empty() {
                state.selected_compare_output = Some(state.compare_queue[0].output_path.clone());
            }
            (
                state.compare_queue.clone(),
                state.selected_compare_output.clone(),
            )
        };

        load_virtual_async(
            &self.app_state(),
            &items
                .iter()
                .map(|i| i.output_path.clone())
                .collect::<Vec<_>>(),
        );

        if let Some(sidebar) = self.imp().sidebar.borrow().as_ref() {
            apply_scope_to_sidebar(&ViewScope::Compare, sidebar);
        }
        if let Some(filmstrip) = self.imp().filmstrip.borrow().as_ref() {
            filmstrip.refresh_virtual();
        }

        if let Some(compare_page) = self.imp().compare_page.borrow().as_ref() {
            if let Some(selected_output) = selected_output.as_deref() {
                if let Some(item) = items
                    .iter()
                    .find(|item| item.output_path.as_path() == selected_output)
                    .cloned()
                {
                    compare_page.set_comparison(item);
                }
            } else if items.is_empty() {
                compare_page.clear();
            }
        }

        if let Some(selected_output) = selected_output {
            let index = {
                let state = self.app_state();
                let mut state = state.borrow_mut();
                let index = (0..state.library.image_count()).find(|&idx| {
                    state
                        .library
                        .entry_at(idx)
                        .map(|entry: ImageEntry| entry.path() == selected_output)
                        .unwrap_or(false)
                });
                state.library.selected_index = index;
                index
            };
            if let (Some(index), Some(filmstrip)) = (index, self.imp().filmstrip.borrow().as_ref())
            {
                filmstrip.navigate_to(index);
            }
        }
    }

    fn restore_view_for_scope(&self, scope: ViewScope) {
        match scope {
            ViewScope::Folder(path) => {
                if let Some(cb) = self.imp().open_folder_cb.borrow().as_ref() {
                    cb(path);
                }
            }
            _ => {
                if let Some(cb) = self.imp().refresh_sidebar_collections_cb.borrow().as_ref() {
                    cb();
                }
            }
        }
    }

    pub fn handle_compare_selection_change(&self, output_path: PathBuf) {
        let state_rc = self.app_state();
        let item = {
            let mut state = state_rc.borrow_mut();
            state.selected_compare_output = Some(output_path.clone());
            state
                .compare_queue
                .iter()
                .find(|i| i.output_path == output_path)
                .cloned()
        };
        if let Some(item) = item {
            if let Some(compare_page) = self.imp().compare_page.borrow().as_ref() {
                compare_page.set_comparison(item);
            }
        }
    }

    pub fn remove_from_compare_queue(&self, output_path: &Path) {
        let state_rc = self.app_state();
        let (empty, next_selected, pipeline_id) = {
            let mut state = state_rc.borrow_mut();
            let removed_pos = state
                .compare_queue
                .iter()
                .position(|i| i.output_path == output_path);
            let pipeline_id = removed_pos.map(|pos| state.compare_queue[pos].pipeline_id);

            if let Some(pos) = removed_pos {
                state.compare_queue.remove(pos);
            }

            if state.compare_queue.is_empty() {
                state.selected_compare_output = None;
                (true, None, pipeline_id)
            } else {
                let next_selected = if state
                    .selected_compare_output
                    .as_deref()
                    .map(|path| path == output_path)
                    .unwrap_or(false)
                {
                    let idx = removed_pos.unwrap_or(0).min(state.compare_queue.len() - 1);
                    Some(state.compare_queue[idx].output_path.clone())
                } else {
                    state.selected_compare_output.clone()
                };
                state.selected_compare_output = next_selected.clone();
                (false, next_selected, pipeline_id)
            }
        };

        if let Some(pid) = pipeline_id {
            let state = state_rc.borrow();
            if let Some(idx) = state.library_index.as_ref() {
                let _ = idx.delete_pipeline(pid);
            }
        }

        if empty {
            if let Some(compare_page) = self.imp().compare_page.borrow().as_ref() {
                compare_page.clear();
            }
            self.exit_compare_mode_internal(true);
        } else {
            self.refresh_compare_view();
            if let Some(next_selected) = next_selected {
                self.handle_compare_selection_change(next_selected);
            }
        }
    }
}
