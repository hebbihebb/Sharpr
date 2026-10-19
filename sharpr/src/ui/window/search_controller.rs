//! Scoped browsing keeps an immutable candidate set while query results change.
use super::*;

type BrowseTask = Box<dyn FnOnce(&mut BrowseCaches) -> Result<BrowseResult, String> + Send>;

/// Rows for the filmstrip, plus how visual search went when it ran.
pub(super) struct BrowseResult {
    paths: Vec<PathBuf>,
    visual: Option<VisualSummary>,
    /// The shown results that match strongly: file-name matches, and
    /// visual results that clear `STRONG_RESULT`.
    strong: Vec<PathBuf>,
}

#[derive(Clone)]
pub(super) enum VisualSummary {
    Ranked {
        name_matches: usize,
        /// Coverage over every candidate, file-name matches included.
        indexed: usize,
        eligible: usize,
        strength: Option<crate::visual_search::search::Strength>,
        /// More visual results wait behind "Show More".
        more: bool,
        /// The good matches are in the chosen sort order (there were strong
        /// visual matches); otherwise the closest come first.
        sorted: bool,
        /// Images left out for looking like a negative word.
        excluded: usize,
        /// Why a negative word's visual part couldn't be applied.
        not_applied: Option<String>,
    },
    /// Visual search couldn't run; file names were matched instead.
    Unavailable(String),
    /// Show Similar (item 28, D44). `found` is `None` when the image itself
    /// isn't indexed yet.
    Similar {
        level: crate::visual_search::similar::Level,
        found: Option<usize>,
        /// Versions of the image in its own folder, left out of Similar.
        copies_left_out: usize,
        /// More wait behind "Show More".
        more: bool,
        indexed: usize,
        eligible: usize,
        /// Searched the whole library rather than the folder or bucket.
        whole_library: bool,
    },
}

/// One visual search's complete ranking. "Show More" pages through it
/// without ranking again, so the images shown stay shown while indexing
/// adds vectors; each page is sorted in the chosen order. A new query or
/// sort order replaces it.
struct RankedSnapshot {
    /// The browse session, the search box's text and the sort order it
    /// answers.
    key: (u64, String, crate::model::library::SortOrder),
    by_name: Vec<PathBuf>,
    /// Visual results, closest first, without the file-name matches.
    visual: Vec<PathBuf>,
    /// Visual results that are strong matches on their own.
    strong: std::collections::HashSet<PathBuf>,
    summary: VisualSummary,
    /// What the sort needs to know about each candidate.
    entries: std::collections::HashMap<PathBuf, crate::model::library::RawImageEntry>,
    order: crate::model::library::SortOrder,
}

impl RankedSnapshot {
    /// One page of results (user, October 19): the file-name matches and the
    /// strong visual matches, sorted in the chosen order, then weaker
    /// matches closest first, which "Show More" adds a page at a time.
    /// Every strong match is in the first block. `limit` grows by one page
    /// per "Show More", starting at one page, and controls only weak results.
    /// With no strong visual match the closest images show at once, closest
    /// first, so a search never comes back empty.
    fn page(&self, limit: usize) -> BrowseResult {
        let (strong, weak): (Vec<&PathBuf>, Vec<&PathBuf>) = self
            .visual
            .iter()
            .partition(|path| self.strong.contains(*path));
        let good: Vec<PathBuf> = self
            .by_name
            .iter()
            .chain(strong.iter().copied())
            .cloned()
            .collect();
        let weak_shown = if strong.is_empty() {
            limit
        } else {
            limit.saturating_sub(crate::visual_search::search::PAGE)
        };
        let mut paths = self.sorted(good.clone());
        paths.extend(weak.iter().take(weak_shown).map(|path| (*path).clone()));
        let mut summary = self.summary.clone();
        if let VisualSummary::Ranked { more, sorted, .. } = &mut summary {
            *more = weak.len() > weak_shown;
            *sorted = !strong.is_empty();
        }
        BrowseResult {
            paths,
            visual: Some(summary),
            strong: good,
        }
    }

    /// `paths` in the chosen sort order.
    fn sorted(&self, paths: Vec<PathBuf>) -> Vec<PathBuf> {
        let mut entries: Vec<_> = paths
            .iter()
            .filter_map(|path| self.entries.get(path).cloned())
            .collect();
        if entries.len() != paths.len() {
            return paths;
        }
        crate::model::library::sort_raw_entries(&mut entries, self.order);
        entries.into_iter().map(|entry| entry.path).collect()
    }
}

/// A whole-library walk: its session, roots, disabled folders and paths.
type LibraryWalk = (u64, Vec<PathBuf>, Vec<PathBuf>, Vec<PathBuf>);

/// State the browse worker keeps between queries.
#[derive(Default)]
pub(super) struct BrowseCaches {
    pub dimensions: crate::model::browse::DimensionCache,
    /// Whole-library file list for one search session, keyed by session,
    /// library roots and disabled folders. Walking the library tree on every
    /// keystroke grows with library size.
    library_paths: Option<LibraryWalk>,
    ranked: Option<RankedSnapshot>,
}

impl BrowseCaches {
    fn library_paths(
        &mut self,
        session: u64,
        roots: &[PathBuf],
        disabled: &[PathBuf],
        superseded: &dyn Fn() -> bool,
    ) -> Result<Vec<PathBuf>, String> {
        if let Some((cached_session, cached_roots, cached_disabled, paths)) = &self.library_paths {
            if *cached_session == session && cached_roots == roots && cached_disabled == disabled {
                return Ok(paths.clone());
            }
        }
        let paths = crate::model::library::collect_library_image_paths_cancellable(
            roots,
            FolderMode::DrillDown,
            None,
            disabled,
            superseded,
        );
        // A walk cut short by a newer query is incomplete: never cache it.
        if superseded() {
            return Err("Search superseded".into());
        }
        self.library_paths = Some((session, roots.to_vec(), disabled.to_vec(), paths.clone()));
        Ok(paths)
    }
}
struct BrowseJob {
    generation: u64,
    task: BrowseTask,
    response: async_channel::Sender<Result<BrowseResult, String>>,
}
pub(super) struct BrowseWorker {
    requests: async_channel::Sender<BrowseJob>,
}
impl BrowseWorker {
    pub fn new(generation: Arc<std::sync::atomic::AtomicU64>) -> Self {
        let (tx, rx) = async_channel::unbounded::<BrowseJob>();
        std::thread::spawn(move || {
            let mut cache = BrowseCaches::default();
            while let Ok(mut job) = rx.recv_blocking() {
                while let Ok(newer) = rx.try_recv() {
                    job = newer;
                }
                if generation.load(std::sync::atomic::Ordering::Relaxed) != job.generation {
                    continue;
                }
                let result = (job.task)(&mut cache);
                let _ = job.response.send_blocking(result);
            }
        });
        Self { requests: tx }
    }
    fn submit(
        &self,
        generation: u64,
        response: async_channel::Sender<Result<BrowseResult, String>>,
        task: BrowseTask,
    ) {
        let _ = self.requests.try_send(BrowseJob {
            generation,
            task,
            response,
        });
    }
}

#[derive(Clone)]
pub(crate) struct BrowseSnapshot {
    pub scope: ViewScope,
    pub folder: Option<PathBuf>,
    pub entries: Vec<ImageEntry>,
    pub selected: Option<PathBuf>,
    pub action_selection: std::collections::HashSet<PathBuf>,
    pub scroll: f64,
    pub sort_order: crate::model::library::SortOrder,
    pub members_changed: bool,
}

#[derive(Clone, Default)]
pub(crate) struct BrowseOptions {
    pub whole_library: bool,
    /// Chips taken out of the search box (decision D16).
    pub chips: crate::model::search_query::ChipFilters,
    /// Show Similar's image and level (item 28, D44): the search ranks the
    /// images in scope by how like it they look instead of by words.
    pub similar: Option<(PathBuf, crate::visual_search::similar::Level)>,
}
impl BrowseOptions {
    pub(super) fn has_filters(&self) -> bool {
        self.whole_library || !self.chips.is_empty() || self.similar.is_some()
    }
}

impl SharprWindow {
    /// A bucket write is independent of the user's current search. Re-read
    /// membership, then rerun the latest inputs without clearing them or
    /// publishing an older worker's unfiltered snapshot.
    pub(crate) fn refresh_current_bucket(&self, id: i64) {
        if file_actions_blocked(self.upcast_ref())
            || self.app_state().borrow().scope != ViewScope::Collection(id)
        {
            return;
        }
        let (index, roots, disabled) = {
            let state = self.app_state();
            let state = state.borrow();
            (
                state.library_index.clone(),
                state.settings.library_roots(),
                state.disabled_folders.clone(),
            )
        };
        let Some(index) = index else { return };
        let generation = self.imp().bucket_open_generation.get().wrapping_add(1);
        self.imp().bucket_open_generation.set(generation);
        self.imp().bucket_open_pending.set(Some(generation));
        let search = self.imp().inline_search_generation.get().wrapping_add(1);
        self.imp().inline_search_generation.set(search);
        self.imp()
            .browse_generation
            .store(search, std::sync::atomic::Ordering::Relaxed);
        let (tx, rx) = async_channel::bounded(1);
        std::thread::spawn(move || {
            let _ = tx.send_blocking(crate::buckets::view(&index, id, &roots, &disabled));
        });
        let weak = self.downgrade();
        glib::MainContext::default().spawn_local(async move {
            let Ok(result) = rx.recv().await else { return };
            let Some(window) = weak.upgrade() else { return };
            if window.imp().bucket_open_pending.get() == Some(generation) {
                window.imp().bucket_open_pending.set(None);
            }
            if window.imp().bucket_open_generation.get() != generation
                || window.app_state().borrow().scope != ViewScope::Collection(id)
                || file_actions_blocked(window.upcast_ref())
            {
                return;
            }
            let paths = match result {
                Ok(view) => view.paths,
                Err(error) => {
                    window.add_toast(libadwaita::Toast::new(&format!(
                        "Could not refresh bucket: {error}"
                    )));
                    return;
                }
            };
            let paths = {
                let state = window.app_state();
                let state = state.borrow();
                filter_paths_for_library(
                    paths,
                    &state.settings.library_roots(),
                    &state.disabled_folders,
                )
            };
            window.capture_browse_view();
            if let Some(snapshot) = window.imp().browse_snapshot.borrow_mut().as_mut() {
                let old: HashMap<_, _> = snapshot
                    .entries
                    .iter()
                    .map(|e| (e.path(), e.clone()))
                    .collect();
                snapshot.entries = paths
                    .into_iter()
                    .map(|path| {
                        old.get(&path)
                            .cloned()
                            .unwrap_or_else(|| ImageEntry::new(path))
                    })
                    .collect();
                snapshot.folder = None;
                snapshot.members_changed = true;
            }
            // Invalidate cached rankings and whole-library candidate lists.
            window
                .imp()
                .browse_session
                .set(window.imp().browse_session.get().wrapping_add(1));
            let query = window
                .imp()
                .inline_search_entry
                .borrow()
                .as_ref()
                .map(|e| e.text().to_string())
                .unwrap_or_default();
            // The existing extend path preserves the current selection/scroll;
            // the new session forces fresh ranking rather than reusing a page.
            window.run_browse_query_with_limit(query, true, true);
        });
    }

    /// After a fingerprint pass changed what members resolve to (a member
    /// moved outside Sharpr was found, or one went missing), or a move inside
    /// Sharpr gave a member a new path: re-reads the
    /// open bucket on a worker and refreshes it only if its images differ
    /// from those it holds, keeping the selection and the search.
    pub(crate) fn refresh_bucket_if_members_changed(&self, id: i64) {
        if self.imp().bucket_open_pending.get().is_some()
            || self.app_state().borrow().scope != ViewScope::Collection(id)
        {
            return;
        }
        let (index, roots, disabled) = {
            let state = self.app_state();
            let state = state.borrow();
            (
                state.library_index.clone(),
                state.settings.library_roots(),
                state.disabled_folders.clone(),
            )
        };
        let Some(index) = index else { return };
        let generation = self.imp().bucket_open_generation.get();
        let (tx, rx) = async_channel::bounded(1);
        std::thread::spawn(move || {
            let paths = crate::buckets::view(&index, id, &roots, &disabled)
                .map(|view| filter_paths_for_library(view.paths, &roots, &disabled));
            let _ = tx.send_blocking(paths);
        });
        let weak = self.downgrade();
        glib::MainContext::default().spawn_local(async move {
            let Ok(Ok(paths)) = rx.recv().await else {
                return;
            };
            let Some(window) = weak.upgrade() else { return };
            if window.imp().bucket_open_generation.get() != generation
                || window.imp().bucket_open_pending.get().is_some()
                || window.app_state().borrow().scope != ViewScope::Collection(id)
            {
                return;
            }
            // With a search running, the bucket's images are in the snapshot.
            let held: std::collections::HashSet<PathBuf> =
                match window.imp().browse_snapshot.borrow().as_ref() {
                    Some(snapshot) => snapshot.entries.iter().map(|e| e.path()).collect(),
                    None => {
                        let state = window.app_state();
                        let state = state.borrow();
                        (0..state.library.image_count())
                            .filter_map(|i| state.library.entry_at(i))
                            .map(|e| e.path())
                            .collect()
                    }
                };
            if paths.len() != held.len() || paths.iter().any(|p| !held.contains(p)) {
                window.refresh_current_bucket(id);
            }
        });
    }

    pub(super) fn capture_browse_view(&self) {
        if self.imp().browse_snapshot.borrow().is_some() {
            return;
        }
        let st = self.imp().state.borrow();
        let entries: Vec<_> = (0..st.library.image_count())
            .filter_map(|i| st.library.entry_at(i))
            .collect();
        let selected = st
            .library
            .selected_index
            .and_then(|i| st.library.entry_at(i))
            .map(|e| e.path());
        let scroll = self
            .imp()
            .filmstrip
            .borrow()
            .as_ref()
            .map(|f| f.scroll_position())
            .unwrap_or(0.0);
        self.imp()
            .browse_session
            .set(self.imp().browse_session.get().wrapping_add(1));
        *self.imp().browse_snapshot.borrow_mut() = Some(BrowseSnapshot {
            scope: st.scope.clone(),
            folder: st.library.current_folder.clone(),
            entries,
            selected,
            action_selection: st.selected_paths.clone(),
            scroll,
            sort_order: st.sort_order,
            members_changed: false,
        });
    }

    pub(super) fn restore_browse_view(&self) {
        let Some(snapshot) = self.imp().browse_snapshot.borrow_mut().take() else {
            return;
        };
        let disabled = self.imp().state.borrow().disabled_folders.clone();
        let removed = self.imp().state.borrow().library.removed_paths.clone();
        let entries: Vec<_> = snapshot
            .entries
            .into_iter()
            .filter(|e| !path_is_disabled(&e.path(), &disabled) && !removed.contains(&e.path()))
            .collect();
        {
            let mut st = self.imp().state.borrow_mut();
            st.scope = snapshot.scope.clone();
            if !snapshot.members_changed {
                st.library.restore_view(&entries, snapshot.folder.clone());
            }
        }
        if snapshot.members_changed {
            load_virtual_async(
                &self.app_state(),
                &entries.iter().map(|e| e.path()).collect::<Vec<_>>(),
            );
        }
        {
            let mut st = self.imp().state.borrow_mut();
            st.selected_paths = snapshot
                .action_selection
                .into_iter()
                .filter(|p| st.library.index_of_path(p).is_some())
                .collect();
        }
        if let Some(sidebar) = self.imp().sidebar.borrow().as_ref() {
            apply_scope_to_sidebar(&snapshot.scope, sidebar);
        }
        self.present_browse_paths(snapshot.selected.as_deref());
        if let Some(f) = self.imp().filmstrip.borrow().as_ref() {
            f.refresh_multi_selection_visuals();
        }
        if let Some(filmstrip) = self.imp().filmstrip.borrow().as_ref() {
            filmstrip.restore_scroll_position(snapshot.scroll);
        }
        if self.imp().state.borrow().sort_order != snapshot.sort_order {
            self.run_browse_query(String::new(), true);
        }
    }

    /// Picks the filmstrip's empty-state title for what is being shown.
    pub(super) fn update_empty_title(&self) {
        let title = if self.imp().browse_snapshot.borrow().is_some() {
            "No matching images"
        } else {
            match self.imp().state.borrow().scope {
                ViewScope::Folder(_) => "No images in this folder",
                ViewScope::Collection(_) => "No images in this bucket",
                _ => "No images",
            }
        };
        if let Some(filmstrip) = self.imp().filmstrip.borrow().as_ref() {
            filmstrip.set_empty_title(title);
        }
    }

    pub(super) fn present_browse_paths(&self, preferred: Option<&Path>) {
        self.update_empty_title();
        let Some(filmstrip) = self.imp().filmstrip.borrow().clone() else {
            return;
        };
        let selected = {
            let mut st = self.imp().state.borrow_mut();
            let selected = preferred
                .and_then(|p| st.library.index_of_path(p))
                .or_else(|| (st.library.image_count() > 0).then_some(0));
            st.library.selected_index = selected;
            selected
        };
        filmstrip.refresh_virtual();
        if let Some(selected) = selected {
            filmstrip.navigate_to(selected);
        } else if let Some(viewer) = self.imp().viewer.borrow().as_ref() {
            viewer.clear();
        }
    }

    /// Moves filter words the user has finished typing out of the search box
    /// and into chips (decision D16). Pressing Enter (`finished`) finishes
    /// the last word too, so `tower -water` shows its "Not water" chip.
    pub(super) fn take_completed_search_chips(&self, entry: &gtk4::SearchEntry, finished: bool) {
        let text = entry.text();
        let (chips, remaining) = if finished {
            let (chips, remaining) =
                crate::model::search_query::take_completed_chips(&format!("{text} "));
            (chips, remaining.trim_end().to_string())
        } else {
            crate::model::search_query::take_completed_chips(text.as_str())
        };
        if chips.is_empty() {
            return;
        }
        {
            let mut options = self.imp().browse_options.borrow_mut();
            for chip in chips {
                if !options.chips.chips.contains(&chip) {
                    options.chips.chips.push(chip);
                }
            }
        }
        self.imp().inline_search_suppressed.set(true);
        entry.set_text(&remaining);
        entry.set_position(-1);
        self.imp().inline_search_suppressed.set(false);
        self.rebuild_search_chips();
    }

    /// Shows the chip bar while it has chips or suggestions to offer.
    fn sync_search_chip_bar(&self) {
        let has_chips = {
            let options = self.imp().browse_options.borrow();
            !options.chips.is_empty() || options.similar.is_some()
        };
        let has_suggestions = self
            .imp()
            .search_suggestions
            .borrow()
            .as_ref()
            .is_some_and(|s| s.is_visible());
        let has_status = self
            .imp()
            .search_status
            .borrow()
            .as_ref()
            .is_some_and(|status| status.bar.is_visible());
        if let Some(bar) = self.imp().search_chip_bar.borrow().as_ref() {
            bar.set_reveal_child(has_chips || has_suggestions || has_status);
        }
    }

    /// Offers filter words that complete the word being typed (D16). Clicking
    /// one completes it with a space, so the search turns it into a chip.
    pub(super) fn rebuild_search_suggestions(&self, text: &str) {
        let Some(container) = self.imp().search_suggestions.borrow().clone() else {
            return;
        };
        while let Some(child) = container.first_child() {
            container.remove(&child);
        }
        let suggestions = crate::model::search_query::filter_suggestions(text);
        container.set_visible(!suggestions.is_empty());
        for suggestion in suggestions {
            let button = gtk4::Button::with_label(suggestion);
            button.add_css_class("flat");
            button.set_tooltip_text(Some("Use this filter"));
            let weak = self.downgrade();
            button.connect_clicked(move |_| {
                let Some(win) = weak.upgrade() else { return };
                let entry = win.imp().inline_search_entry.borrow().clone();
                if let Some(entry) = entry {
                    let completed = crate::model::search_query::apply_suggestion(
                        entry.text().as_str(),
                        suggestion,
                    );
                    entry.set_text(&completed);
                    // Focusing selects all text; move the cursor afterwards.
                    entry.set_can_focus(true);
                    entry.grab_focus();
                    entry.set_position(-1);
                }
            });
            container.append(&button);
            if let Some(child) = button.parent() {
                child.set_focusable(false);
            }
        }
        self.sync_search_chip_bar();
    }

    /// Shows one chip per typed filter. The label turns the chip back into a
    /// search word; the close button removes the filter.
    pub(super) fn rebuild_search_chips(&self) {
        let Some(container) = self.imp().search_chips.borrow().clone() else {
            return;
        };
        while let Some(child) = container.first_child() {
            container.remove(&child);
        }
        let chips = self.imp().browse_options.borrow().chips.chips.clone();
        let similar = self.imp().browse_options.borrow().similar.clone();
        container.set_visible(!chips.is_empty() || similar.is_some());
        self.sync_search_chip_bar();
        if let Some((path, level)) = similar {
            container.append(&self.similar_chip(&path, level));
            if let Some(child) = container.last_child() {
                child.set_focusable(false);
            }
        }
        for chip in chips {
            let pill = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
            pill.add_css_class("linked");
            // Look prototype: a lowercase monospace tag; minus words get
            // their own tone.
            pill.add_css_class("sharpr-filter-chip");
            if matches!(
                chip,
                crate::model::search_query::Chip::Not(_)
                    | crate::model::search_query::Chip::Without(_)
                    | crate::model::search_query::Chip::WithoutStrictly(_)
            ) {
                pill.add_css_class("negative");
            }
            pill.set_halign(gtk4::Align::Start);
            let label = gtk4::Button::with_label(&chip.label().to_lowercase());
            label.set_tooltip_text(Some("Search for this as a word instead"));
            let remove = gtk4::Button::from_icon_name("window-close-symbolic");
            remove.set_tooltip_text(Some("Remove filter"));
            remove.update_property(&[gtk4::accessible::Property::Label(&format!(
                "Remove {} filter",
                chip.label()
            ))]);
            pill.append(&label);
            pill.append(&remove);
            container.append(&pill);
            if let Some(child) = pill.parent() {
                // Only the buttons take keyboard focus, not the wrapping cell.
                child.set_focusable(false);
            }

            let weak = self.downgrade();
            let label_chip = chip.clone();
            label.connect_clicked(move |_| {
                let chip = label_chip.clone();
                let Some(win) = weak.upgrade() else { return };
                win.imp()
                    .browse_options
                    .borrow_mut()
                    .chips
                    .chips
                    .retain(|c| *c != chip);
                win.rebuild_search_chips();
                let entry = win.imp().inline_search_entry.borrow().clone();
                if let Some(entry) = entry {
                    let text = entry.text().to_string();
                    let literal = chip.as_literal();
                    let joined = if text.trim().is_empty() {
                        literal
                    } else {
                        format!("{} {literal}", text.trim_end())
                    };
                    // The changed text runs the search through the entry.
                    entry.set_text(&joined);
                    entry.set_position(-1);
                }
            });
            let weak = self.downgrade();
            remove.connect_clicked(move |_| {
                let Some(win) = weak.upgrade() else { return };
                win.imp()
                    .browse_options
                    .borrow_mut()
                    .chips
                    .chips
                    .retain(|c| *c != chip);
                win.rebuild_search_chips();
                let query = win
                    .imp()
                    .inline_search_entry
                    .borrow()
                    .as_ref()
                    .map(|e| e.text().to_string())
                    .unwrap_or_default();
                win.run_scoped_search(query);
            });
        }
    }

    pub(super) fn run_scoped_search(&self, query: String) {
        self.run_browse_query(query, false);
    }

    /// Shows the next page of visual-search results for the current words.
    pub(super) fn show_more_visual_results(&self) {
        let limit = self.imp().visual_limit.get() + crate::visual_search::search::PAGE;
        self.imp().visual_limit.set(limit);
        let query = self
            .imp()
            .inline_search_entry
            .borrow()
            .as_ref()
            .map(|e| e.text().to_string())
            .unwrap_or_default();
        self.run_browse_query_with_limit(query, false, true);
    }

    /// Says how a visual search went, or hides the line.
    pub(super) fn show_search_status(&self, summary: Option<&VisualSummary>) {
        let Some(status) = self.imp().search_status.borrow().clone() else {
            return;
        };
        let text = summary.map(visual_summary_text);
        status.bar.set_visible(text.is_some());
        status.label.set_text(text.as_deref().unwrap_or_default());
        status.more.set_visible(matches!(
            summary,
            Some(
                VisualSummary::Ranked { more: true, .. }
                    | VisualSummary::Similar { more: true, .. }
            )
        ));
        let ranked = matches!(
            summary,
            Some(VisualSummary::Ranked { .. } | VisualSummary::Similar { .. })
        );
        status.add.set_visible(ranked);
        if !ranked {
            self.imp().search_strong.borrow_mut().clear();
        }
        self.sync_search_chip_bar();
    }

    /// Shows how many images are selected beside the search status.
    pub(super) fn show_search_selection(&self, selected: usize) {
        if let Some(status) = self.imp().search_status.borrow().as_ref() {
            // Called from selection callbacks: the state may be borrowed.
            let explicit = self
                .imp()
                .state
                .try_borrow()
                .is_ok_and(|st| st.selection_explicit);
            status
                .selected
                .set_visible(selected > 1 || (explicit && selected == 1));
            status.selected.set_text(&format!("{selected} selected"));
        }
    }

    /// "Add to Bucket…": the selection when there is one; otherwise the
    /// user picks the strong matches or everything shown. The bucket picker
    /// (with "New Bucket…") follows, so three actions make a bucket.
    pub(super) fn add_search_results_to_bucket(&self) {
        let Some(filmstrip) = self.imp().filmstrip.borrow().clone() else {
            return;
        };
        let chosen = {
            let st = self.imp().state.borrow();
            st.selected_paths.len() > 1 || (st.selection_explicit && !st.selected_paths.is_empty())
        };
        if chosen {
            filmstrip.request_add_selection_to_bucket();
            return;
        }
        let shown: Vec<PathBuf> = {
            let st = self.imp().state.borrow();
            (0..st.library.image_count())
                .filter_map(|i| st.library.entry_at(i).map(|e| e.path()))
                .collect()
        };
        let strong = self.imp().search_strong.borrow().clone();
        // Look prototype: a small popup at the pointer (I7). The bucket
        // picker comes next.
        let heading = gtk4::Label::new(Some("Add which results?"));
        heading.add_css_class("heading");
        heading.set_xalign(0.0);
        let strong_btn = gtk4::Button::with_label(&format!("Strong Matches ({})", strong.len()));
        strong_btn.set_sensitive(!strong.is_empty());
        strong_btn.set_tooltip_text(Some(
            "File-name matches and images that match these words well on their own",
        ));
        let all_btn = gtk4::Button::with_label(&format!("All Shown ({})", shown.len()));
        let default = if strong.is_empty() {
            &all_btn
        } else {
            &strong_btn
        };
        default.add_css_class("suggested-action");
        let content = gtk4::Box::new(gtk4::Orientation::Vertical, 6);
        content.append(&heading);
        content.append(&strong_btn);
        content.append(&all_btn);
        let popover = self.popover_at_pointer(&content);
        popover.set_default_widget(Some(default));
        for (button, paths) in [(strong_btn, strong), (all_btn, shown)] {
            let filmstrip = filmstrip.clone();
            let popover = popover.downgrade();
            button.connect_clicked(move |_| {
                if let Some(popover) = popover.upgrade() {
                    popover.popdown();
                }
                filmstrip.request_add_paths_to_bucket(paths.clone());
            });
        }
    }

    pub(super) fn run_browse_query(&self, query: String, sort_only: bool) {
        self.imp()
            .visual_limit
            .set(crate::visual_search::search::PAGE);
        self.run_browse_query_with_limit(query, sort_only, false);
    }

    /// `extend` shows the next page of the current visual ranking instead of
    /// ranking again.
    fn run_browse_query_with_limit(&self, query: String, sort_only: bool, extend: bool) {
        // A delayed search-entry signal must not replace a Compare/crop
        // canvas after the user has entered that mode.
        if super::file_actions_blocked(self.upcast_ref()) {
            return;
        }
        // Until a bucket's members arrive, the filmstrip still holds the previous
        // view; searching it would label those images as the bucket. The bucket
        // open runs the current query when it completes.
        if self.imp().bucket_open_pending.get().is_some()
            && matches!(self.imp().state.borrow().scope, ViewScope::Collection(_))
        {
            return;
        }
        let generation = self.imp().inline_search_generation.get().wrapping_add(1);
        self.imp().inline_search_generation.set(generation);
        self.imp()
            .browse_generation
            .store(generation, std::sync::atomic::Ordering::Relaxed);
        let options = self.imp().browse_options.borrow().clone();
        if query.trim().is_empty() && !options.has_filters() && !sort_only {
            self.show_search_status(None);
            self.restore_browse_view();
            return;
        }
        self.capture_browse_view();
        let (candidates, roots, disabled, scope, order, index) = {
            let st = self.imp().state.borrow();
            let snapshot = self.imp().browse_snapshot.borrow();
            let Some(snapshot) = snapshot.as_ref() else {
                return;
            };
            (
                snapshot
                    .entries
                    .iter()
                    .map(|e| e.path())
                    .collect::<Vec<_>>(),
                st.settings.library_roots(),
                st.disabled_folders.clone(),
                st.scope.clone(),
                st.sort_order,
                st.library_index.clone(),
            )
        };
        // Chips already taken out of the box, plus filter words still being typed.
        let parsed = crate::model::search_query::parse(&query);
        let mut chips = options.chips.clone();
        for chip in parsed.chips {
            if !chips.chips.contains(&chip) {
                chips.chips.push(chip);
            }
        }
        let text = parsed.text;
        let session = self.imp().browse_session.get();
        let searcher = self.imp().visual_searcher.borrow().clone();
        let visual_limit = self.imp().visual_limit.get();
        let snapshot_query = query.clone();
        let (tx, rx) = async_channel::bounded(1);
        let expected_query = query.clone();
        let canceled = self.imp().browse_generation.clone();
        self.imp().browse_worker.submit(
            generation,
            tx,
            Box::new(move |cache| {
                let result = (|| -> Result<BrowseResult, String> {
                    let superseded =
                        || canceled.load(std::sync::atomic::Ordering::Relaxed) != generation;
                    if superseded() {
                        return Err("Search superseded".into());
                    }
                    // The sort order is part of the key: a new order sorts again.
                    let snapshot_key = (session, snapshot_query, order);
                    if extend && options.similar.is_none() {
                        if let Some(snapshot) = &cache.ranked {
                            if snapshot.key == snapshot_key {
                                return Ok(snapshot.page(visual_limit));
                            }
                        }
                    }
                    let whole_library = options.whole_library;
                    let candidates = if whole_library {
                        if roots.is_empty() {
                            return Err("No active library".into());
                        }
                        cache.library_paths(session, &roots, &disabled, &superseded)?
                    } else {
                        candidates
                    };
                    let candidates = if whole_library {
                        filter_paths_for_library(candidates, &roots, &disabled)
                    } else {
                        candidates
                            .into_iter()
                            .filter(|p| !path_is_disabled(p, &disabled))
                            .collect()
                    };
                    // Words go to visual search when it can run; otherwise,
                    // and for file names, they match names.
                    // With Show Similar, words only match file names.
                    let visual = (!search_terms(&text).is_empty() && options.similar.is_none())
                        .then_some(searcher.as_ref())
                        .flatten()
                        .map(|searcher| searcher.prepare(&text, &superseded));
                    if superseded() {
                        return Err("Search superseded".into());
                    }
                    let name_only = !matches!(visual, Some(Ok(_)))
                        && (options.similar.is_none() || !search_terms(&text).is_empty());
                    let name_matches: std::collections::HashSet<PathBuf> =
                        match_candidates(&candidates, &text).into_iter().collect();
                    let paths = if name_only {
                        candidates
                            .iter()
                            .filter(|p| name_matches.contains(*p))
                            .cloned()
                            .collect()
                    } else {
                        candidates
                    };
                    let needs_dimensions = chips.needs_dimensions()
                        || matches!(
                            order.field(),
                            crate::model::library::SortField::Aspect
                                | crate::model::library::SortField::WideScreen
                        );
                    let mut raw = Vec::with_capacity(paths.len());
                    let mut stamps = Vec::with_capacity(paths.len());
                    for path in paths {
                        if canceled.load(std::sync::atomic::Ordering::Relaxed) != generation {
                            return Err("Search superseded".into());
                        }
                        let Ok(metadata) = std::fs::metadata(&path) else {
                            continue;
                        };
                        if !metadata.is_file() {
                            continue;
                        }
                        let dimensions = needs_dimensions
                            .then(|| cache.dimensions.get(&path, &metadata))
                            .flatten();
                        if !chips.matches(&path, dimensions) {
                            continue;
                        }
                        let (width, height) = dimensions.unwrap_or((0, 0));
                        stamps.push(crate::fingerprint::file_stamp(&metadata));
                        raw.push(RawImageEntry {
                            filename: path
                                .file_name()
                                .map(|n| n.to_string_lossy().into_owned())
                                .unwrap_or_default(),
                            path,
                            file_size: metadata.len(),
                            modified: metadata.modified().ok(),
                            width,
                            height,
                        });
                    }
                    // Negative words: file names were handled by the chips;
                    // with visual search, the images most like each word go
                    // too, before anything is ranked.
                    let mut fingerprints = None;
                    let mut excluded = 0;
                    let mut not_applied = None;
                    let without = chips.without_words();
                    if let (false, Some(searcher), Some(index)) =
                        (without.is_empty(), searcher.as_ref(), index.as_ref())
                    {
                        let mut alike = std::collections::HashSet::new();
                        for (word, strict) in &without {
                            let share = if *strict {
                                crate::visual_search::search::STRICT_EXCLUDE_SHARE
                            } else {
                                crate::visual_search::search::EXCLUDE_SHARE
                            };
                            match searcher.prepare(word, &superseded) {
                                Ok(negative) => alike.extend(
                                    crate::visual_search::search::most_alike(&negative, share),
                                ),
                                Err(error) => {
                                    not_applied.get_or_insert(error);
                                }
                            }
                            if superseded() {
                                return Err("Search superseded".into());
                            }
                        }
                        if !alike.is_empty() {
                            let map = index.cached_fingerprints().map_err(|e| e.to_string())?;
                            let before = raw.len();
                            let mut kept_stamps = Vec::with_capacity(stamps.len());
                            let mut kept = Vec::with_capacity(raw.len());
                            for (entry, stamp) in raw.into_iter().zip(stamps) {
                                let like = map
                                    .get(entry.path.to_string_lossy().as_ref())
                                    .filter(|(size, mtime, _)| (*size, *mtime) == stamp)
                                    .is_some_and(|(_, _, hash)| alike.contains(hash));
                                if !like {
                                    kept.push(entry);
                                    kept_stamps.push(stamp);
                                }
                            }
                            excluded = before - kept.len();
                            raw = kept;
                            stamps = kept_stamps;
                            fingerprints = Some(map);
                        }
                    }
                    if let Some((target, level)) = &options.similar {
                        let index = index.as_ref().ok_or("Library index is unavailable")?;
                        let fingerprints = match fingerprints {
                            Some(map) => map,
                            None => index.cached_fingerprints().map_err(|e| e.to_string())?,
                        };
                        return similar_result(
                            target,
                            *level,
                            whole_library,
                            visual_limit,
                            raw,
                            &stamps,
                            &fingerprints,
                            searcher.as_ref(),
                            &superseded,
                        );
                    }
                    let prepared = match visual {
                        Some(Ok(prepared)) => prepared,
                        other => {
                            crate::model::library::sort_raw_entries(&mut raw, order);
                            return Ok(BrowseResult {
                                strong: Vec::new(),
                                paths: raw.into_iter().map(|r| r.path).collect(),
                                visual: match other {
                                    Some(Err(error)) => Some(VisualSummary::Unavailable(error)),
                                    _ => None,
                                },
                            });
                        }
                    };
                    // File-name matches first, in the chosen order, then the
                    // closest images. Fingerprints are read fresh for every
                    // query, so newly fingerprinted or edited files count.
                    let index = index.as_ref().ok_or("Library index is unavailable")?;
                    let fingerprints = match fingerprints {
                        Some(map) => map,
                        None => index.cached_fingerprints().map_err(|e| e.to_string())?,
                    };
                    let mut entries = std::collections::HashMap::with_capacity(raw.len());
                    let mut by_name = Vec::new();
                    let mut candidates = Vec::with_capacity(raw.len());
                    for (entry, stamp) in raw.into_iter().zip(stamps) {
                        let fingerprint = fingerprints
                            .get(entry.path.to_string_lossy().as_ref())
                            .filter(|(size, mtime, _)| (*size, *mtime) == stamp)
                            .map(|(_, _, hash)| hash.clone());
                        candidates.push((entry.path.clone(), fingerprint));
                        if name_matches.contains(&entry.path) {
                            by_name.push(entry.path.clone());
                        }
                        entries.insert(entry.path.clone(), entry);
                    }
                    // Coverage and strength count every candidate; only the
                    // listing leaves the file-name matches out of the ranking.
                    let ranked = crate::visual_search::search::rank(&candidates, &prepared);
                    let mut visual = Vec::with_capacity(ranked.paths.len());
                    let mut strong = std::collections::HashSet::new();
                    for (path, is_strong) in ranked.paths.into_iter().zip(ranked.strong) {
                        if name_matches.contains(&path) {
                            continue;
                        }
                        if is_strong {
                            strong.insert(path.clone());
                        }
                        visual.push(path);
                    }
                    let snapshot = RankedSnapshot {
                        key: snapshot_key,
                        summary: VisualSummary::Ranked {
                            name_matches: by_name.len(),
                            indexed: ranked.indexed,
                            eligible: ranked.eligible,
                            strength: ranked.strength,
                            more: false,
                            sorted: false,
                            excluded,
                            not_applied,
                        },
                        by_name,
                        visual,
                        strong,
                        entries,
                        order,
                    };
                    let page = snapshot.page(visual_limit);
                    cache.ranked = Some(snapshot);
                    Ok(page)
                })();
                result
            }),
        );
        let weak = self.downgrade();
        glib::MainContext::default().spawn_local(async move {
            let Ok(result) = rx.recv().await else { return };
            let Some(win) = weak.upgrade() else { return };
            if win.imp().inline_search_generation.get() != generation
                || win.imp().state.borrow().scope != scope
                || win
                    .imp()
                    .inline_search_entry
                    .borrow()
                    .as_ref()
                    .is_some_and(|e| e.text().as_str() != expected_query)
            {
                return;
            }
            match result {
                Ok(BrowseResult {
                    paths,
                    visual,
                    strong,
                }) => {
                    *win.imp().search_strong.borrow_mut() = strong;
                    win.show_search_status(visual.as_ref());
                    let selected = win
                        .imp()
                        .state
                        .borrow()
                        .library
                        .selected_index
                        .and_then(|i| win.imp().state.borrow().library.entry_at(i))
                        .map(|e| e.path());
                    let scroll = win
                        .imp()
                        .filmstrip
                        .borrow()
                        .as_ref()
                        .map(|f| f.scroll_position());
                    win.bump_thumbnail_generation("search.apply");
                    // A new ranking starts at the closest image; "Show More"
                    // keeps the selection and the place in the list.
                    let ranked = matches!(
                        visual,
                        Some(VisualSummary::Ranked { .. } | VisualSummary::Similar { .. })
                    );
                    let selected = if ranked && !extend { None } else { selected };
                    if !extend {
                        win.imp().state.borrow_mut().selected_paths.clear();
                    }
                    load_virtual_async(&win.imp().state, &paths);
                    if extend {
                        let shown: std::collections::HashSet<_> = paths.iter().collect();
                        win.imp()
                            .state
                            .borrow_mut()
                            .selected_paths
                            .retain(|path| shown.contains(path));
                    }
                    win.present_browse_paths(selected.as_deref());
                    if extend {
                        if let (Some(filmstrip), Some(scroll)) =
                            (win.imp().filmstrip.borrow().as_ref(), scroll)
                        {
                            filmstrip.refresh_multi_selection_visuals();
                            filmstrip.restore_scroll_position(scroll);
                        }
                    }
                    if sort_only
                        && expected_query.is_empty()
                        && !win.imp().browse_options.borrow().has_filters()
                    {
                        let snapshot = win.imp().browse_snapshot.borrow_mut().take();
                        if let Some(snapshot) = snapshot {
                            win.imp().state.borrow_mut().library.current_folder = snapshot.folder;
                            if let Some(filmstrip) = win.imp().filmstrip.borrow().as_ref() {
                                filmstrip.restore_scroll_position(snapshot.scroll);
                            }
                        }
                    }
                }
                Err(error) => {
                    win.add_toast(libadwaita::Toast::new(&format!("Search failed: {error}")))
                }
            }
        });
    }
}

/// The Show Similar listing (D44): the image first, then, by `level`, its
/// versions, other takes of its scene, or every other picture closest
/// first, up to `limit`. Versions in the image's own folder are left out of
/// Similar.
/// Without vectors the image shows alone, with the reason.
#[allow(clippy::too_many_arguments)]
fn similar_result(
    target: &Path,
    level: crate::visual_search::similar::Level,
    whole_library: bool,
    limit: usize,
    raw: Vec<RawImageEntry>,
    stamps: &[(u64, i64)],
    fingerprints: &std::collections::HashMap<String, (u64, i64, String)>,
    searcher: Option<&crate::visual_search::search::Searcher>,
    superseded: &dyn Fn() -> bool,
) -> Result<BrowseResult, String> {
    use crate::visual_search::similar::Level;
    let current = |path: &Path, stamp: (u64, i64)| {
        fingerprints
            .get(path.to_string_lossy().as_ref())
            .filter(|(size, mtime, _)| (*size, *mtime) == stamp)
            .map(|(_, _, hash)| hash.clone())
    };
    let candidates: Vec<(PathBuf, Option<String>)> = raw
        .into_iter()
        .zip(stamps)
        .map(|(entry, stamp)| {
            let fingerprint = current(&entry.path, *stamp);
            (entry.path, fingerprint)
        })
        .collect();
    let alone = |visual| BrowseResult {
        strong: Vec::new(),
        paths: vec![target.to_path_buf()],
        visual: Some(visual),
    };
    let target_fingerprint = std::fs::metadata(target)
        .ok()
        .and_then(|m| current(target, crate::fingerprint::file_stamp(&m)));
    let Some(searcher) = searcher else {
        return Ok(alone(VisualSummary::Unavailable(
            "Visual search is off, so there is nothing to compare with".into(),
        )));
    };
    let vectors = match searcher.vectors(superseded) {
        Ok(vectors) => vectors,
        Err(error) if error == crate::visual_search::search::SUPERSEDED => return Err(error),
        Err(error) => return Ok(alone(VisualSummary::Unavailable(error))),
    };
    let indexed = candidates
        .iter()
        .filter(|(_, f)| f.as_ref().is_some_and(|f| vectors.contains_key(f)))
        .count();
    let summary = |found, copies_left_out, more| VisualSummary::Similar {
        level,
        found,
        copies_left_out,
        more,
        indexed,
        eligible: candidates.len(),
        whole_library,
    };
    let ranked = target_fingerprint.as_ref().and_then(|fingerprint| {
        crate::visual_search::similar::neighbours(fingerprint, &candidates, &vectors)
    });
    let (Some(target_fingerprint), Some(ranked)) = (target_fingerprint, ranked) else {
        return Ok(alone(summary(None, 0, false)));
    };
    // Versions of the image (copies, crops, edits); only the few closest
    // need the pixel check, and the image's small version is read once.
    let target_tiny = std::sync::OnceLock::new();
    let tiny = |path: &Path| {
        if path == target {
            target_tiny
                .get_or_init(|| crate::copies::tiny_version(path))
                .clone()
        } else {
            crate::copies::tiny_version(path)
        }
    };
    let readers = crate::copies::Readers {
        size_of: &|_| None,
        tiny: &tiny,
        canceled: &|| false,
        progress: &|_, _| {},
    };
    let cutoff =
        crate::visual_search::similar::close_cutoff(&target_fingerprint, &vectors).unwrap_or(1.0);
    // The levels nest: Exact ⊂ Close ⊂ Similar. Versions in the image's own
    // folder show only under Exact.
    let folder = target.parent();
    let mut listed = Vec::new();
    let mut copies_left_out = 0;
    // Each other picture's rank, from its first file: further files of it
    // (identical copies) share the rank.
    let mut ranks: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for neighbour in ranked {
        if neighbour.path == target {
            continue;
        }
        if superseded() {
            return Err(crate::visual_search::search::SUPERSEDED.into());
        }
        let version = crate::copies::version_of(
            (target, &target_fingerprint),
            (&neighbour.path, &neighbour.fingerprint),
            neighbour.score,
            &readers,
        );
        let keep = if version {
            let here = neighbour.path.parent() == folder;
            if here && level != Level::Exact {
                copies_left_out += 1;
            }
            level == Level::Exact || !here
        } else {
            let next = ranks.len();
            let rank = *ranks.entry(neighbour.fingerprint.clone()).or_insert(next);
            match level {
                Level::Exact => false,
                Level::Close => {
                    crate::visual_search::similar::is_close(rank, neighbour.score, cutoff)
                }
                Level::Similar => true,
            }
        };
        if keep {
            listed.push(neighbour.path);
        }
        // Versions are at least `MAYBE_SAME` alike; Close ends once the
        // nearest few are past and nothing stands out any more.
        let done = match level {
            Level::Exact => neighbour.score < crate::copies::MAYBE_SAME,
            Level::Close => {
                ranks.len() > crate::visual_search::similar::CLOSE_AT_LEAST
                    && neighbour.score < cutoff.min(crate::copies::MAYBE_SAME)
            }
            Level::Similar => false,
        };
        if done {
            break;
        }
    }
    let found = listed.len();
    let more = found > limit;
    listed.truncate(limit);
    let mut paths = vec![target.to_path_buf()];
    paths.extend(listed);
    Ok(BrowseResult {
        strong: Vec::new(),
        paths,
        visual: Some(summary(Some(found), copies_left_out, more)),
    })
}

fn visual_summary_text(summary: &VisualSummary) -> String {
    use crate::visual_search::search::Strength;
    match summary {
        VisualSummary::Similar {
            level,
            found,
            copies_left_out,
            more,
            indexed,
            eligible,
            whole_library,
        } => {
            use crate::visual_search::similar::Level;
            let plural = |n: usize, one: &str, many: &str| match n {
                1 => format!("1 {one}"),
                n => format!("{n} {many}"),
            };
            let mut parts = vec![match (level, found) {
                (_, None) => "This image isn't indexed for visual search yet".to_string(),
                (Level::Exact, Some(0)) if *whole_library => {
                    "No other versions of this picture".to_string()
                }
                (Level::Exact, Some(0)) => {
                    "No other versions here; Whole library looks further".to_string()
                }
                (Level::Exact, Some(n)) => plural(*n, "version", "versions"),
                (Level::Close, Some(0)) => "Nothing else to compare with".to_string(),
                (Level::Close, Some(n)) => {
                    format!(
                        "{}, closest first",
                        plural(*n, "close image", "close images")
                    )
                }
                (Level::Similar, Some(_)) if *more => {
                    "Closest first; Show More adds the next".to_string()
                }
                (Level::Similar, Some(_)) => "Closest first".to_string(),
            }];
            if *copies_left_out > 0 {
                parts.push(format!(
                    "{} in this folder left out",
                    plural(*copies_left_out, "version", "versions")
                ));
            }
            parts.push(if indexed < eligible {
                format!("{indexed} of {eligible} images indexed")
            } else {
                format!("all {eligible} images searched")
            });
            parts.join(" · ")
        }
        VisualSummary::Unavailable(reason) => {
            format!("Matching file names only. {}", reason.trim_end_matches('.'))
        }
        VisualSummary::Ranked {
            name_matches,
            indexed,
            eligible,
            strength,
            excluded,
            not_applied,
            sorted,
            more,
        } => {
            let mut parts = Vec::new();
            if *name_matches > 0 {
                parts.push(format!("{name_matches} by file name"));
            }
            parts.push(
                match strength {
                    Some(Strength::Strong) => "Strong matches",
                    Some(Strength::Some) => "Some good matches",
                    Some(Strength::Weak) => {
                        "Only weak matches: these words may not fit anything here"
                    }
                    None if *indexed == 0 => "None of these images are indexed yet",
                    None => "Closest first",
                }
                .to_string(),
            );
            if *sorted {
                parts.push(if *more {
                    "sorted; Show More adds weaker matches".to_string()
                } else {
                    "sorted".to_string()
                });
            }
            if indexed < eligible {
                parts.push(format!("{indexed} of {eligible} images indexed"));
            } else {
                parts.push(format!("all {eligible} images searched"));
            }
            if *excluded > 0 {
                parts.push(format!("{excluded} left out by the \"not\" words"));
            }
            if let Some(error) = not_applied {
                parts.push(format!(
                    "the \"not\" words could only exclude file names ({})",
                    error.trim_end_matches('.')
                ));
            }
            parts.join(" · ")
        }
    }
}

fn match_candidates(candidates: &[PathBuf], query: &str) -> Vec<PathBuf> {
    let terms = search_terms(query);
    candidates
        .iter()
        .filter(|p| {
            p.file_name().and_then(|n| n.to_str()).is_some_and(|name| {
                let name = name.to_lowercase();
                terms.iter().all(|term| name.contains(term))
            })
        })
        .cloned()
        .collect()
}

impl SharprWindow {
    /// Builds the bar under the viewer header that holds the search chips,
    /// the word suggestions and the visual-search status, and hands it to
    /// the mode header. Called once from `setup`.
    pub(super) fn build_search_chip_bar(
        &self,
        viewer_toolbar: &libadwaita::ToolbarView,
        viewer_header: &libadwaita::HeaderBar,
    ) {
        // Typed filter chips sit in their own bar under the header and wrap,
        // so they never widen the window past its minimum size.
        let search_chips = gtk4::FlowBox::new();
        search_chips.set_selection_mode(gtk4::SelectionMode::None);
        search_chips.set_halign(gtk4::Align::Start);
        search_chips.set_max_children_per_line(32);
        search_chips.set_column_spacing(6);
        search_chips.set_row_spacing(6);
        search_chips.set_margin_start(12);
        search_chips.set_margin_end(12);
        search_chips.set_margin_top(6);
        search_chips.set_margin_bottom(6);
        // Filter words that complete what is being typed (decision D16).
        let search_suggestions = gtk4::FlowBox::new();
        search_suggestions.set_selection_mode(gtk4::SelectionMode::None);
        search_suggestions.set_halign(gtk4::Align::Start);
        search_suggestions.set_max_children_per_line(32);
        search_suggestions.set_column_spacing(4);
        search_suggestions.set_margin_start(12);
        search_suggestions.set_margin_end(12);
        search_suggestions.set_margin_bottom(6);
        search_suggestions.set_visible(false);
        // How much of the scope visual search covered and how well it matched
        // (principle 5), with more results on request.
        let search_status = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
        search_status.set_margin_start(12);
        search_status.set_margin_end(12);
        search_status.set_margin_bottom(6);
        search_status.set_visible(false);
        let search_status_label = gtk4::Label::new(None);
        search_status_label.add_css_class("dim-label");
        search_status_label.set_wrap(true);
        search_status_label.set_xalign(0.0);
        search_status_label.set_hexpand(true);
        let search_more = gtk4::Button::with_label("Show More");
        search_more.add_css_class("flat");
        search_more.set_valign(gtk4::Align::Center);
        search_more.set_tooltip_text(Some("Show the next closest images"));
        {
            let weak = self.downgrade();
            search_more.connect_clicked(move |_| {
                if let Some(win) = weak.upgrade() {
                    win.show_more_visual_results();
                }
            });
        }
        let search_selected = gtk4::Label::new(None);
        search_selected.add_css_class("dim-label");
        search_selected.add_css_class("numeric");
        search_selected.set_visible(false);
        let search_add = gtk4::Button::with_label("Add to Bucket…");
        search_add.add_css_class("flat");
        search_add.set_valign(gtk4::Align::Center);
        search_add.set_tooltip_text(Some("Add these results to a new or existing bucket"));
        search_add.set_visible(false);
        {
            let weak = self.downgrade();
            search_add.connect_clicked(move |_| {
                if let Some(win) = weak.upgrade() {
                    win.add_search_results_to_bucket();
                }
            });
        }
        search_status.append(&search_status_label);
        search_status.append(&search_selected);
        search_status.append(&search_add);
        search_status.append(&search_more);
        let chip_bar = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        chip_bar.append(&search_chips);
        chip_bar.append(&search_suggestions);
        chip_bar.append(&search_status);
        *self.imp().search_status.borrow_mut() = Some(SearchStatus {
            bar: search_status,
            label: search_status_label,
            selected: search_selected,
            add: search_add,
            more: search_more,
        });
        let search_chips_revealer = gtk4::Revealer::new();
        search_chips_revealer.set_transition_type(gtk4::RevealerTransitionType::SlideDown);
        search_chips_revealer.set_child(Some(&chip_bar));
        viewer_toolbar.add_top_bar(&search_chips_revealer);
        *self.imp().search_chips.borrow_mut() = Some(search_chips);
        *self.imp().search_suggestions.borrow_mut() = Some(search_suggestions);
        *self.imp().mode_header.borrow_mut() = Some(mode_header::ModeHeader::new(
            viewer_header,
            &search_chips_revealer,
        ));
        *self.imp().search_chip_bar.borrow_mut() = Some(search_chips_revealer);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whole_library_list_is_reused_within_a_search_session_only() {
        let root = std::env::temp_dir().join(format!(
            "sharpr-browse-cache-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("a.jpg"), b"a").unwrap();
        let mut caches = BrowseCaches::default();
        let current = || false;

        assert_eq!(
            caches
                .library_paths(1, std::slice::from_ref(&root), &[], &current)
                .unwrap()
                .len(),
            1
        );
        std::fs::write(root.join("b.jpg"), b"b").unwrap();
        assert_eq!(
            caches
                .library_paths(1, std::slice::from_ref(&root), &[], &current)
                .unwrap()
                .len(),
            1,
            "same session reuses the walk"
        );
        assert!(caches
            .library_paths(2, std::slice::from_ref(&root), &[], &|| true)
            .is_err());
        assert_eq!(
            caches
                .library_paths(2, std::slice::from_ref(&root), &[], &current)
                .unwrap()
                .len(),
            2,
            "a new session walks again; the cancelled walk was not kept"
        );

        std::fs::remove_dir_all(root).unwrap();
    }
    fn snapshot(strong: &[&str]) -> RankedSnapshot {
        use crate::model::library::{RawImageEntry, SortOrder};
        let path = |n: &str| PathBuf::from(format!("/l/{n}.jpg"));
        // Closest first: z, b, y, a, x. The file-name match is "m-tower".
        let visual: Vec<PathBuf> = ["z", "b", "y", "a", "x"].map(path).to_vec();
        let by_name = vec![path("m-tower")];
        let entries = by_name
            .iter()
            .chain(&visual)
            .map(|path| {
                let entry = RawImageEntry {
                    path: path.clone(),
                    filename: path.file_name().unwrap().to_string_lossy().into_owned(),
                    file_size: 1,
                    modified: None,
                    width: 1,
                    height: 1,
                };
                (path.clone(), entry)
            })
            .collect();
        RankedSnapshot {
            key: (1, "tower".into(), SortOrder::NameAsc),
            by_name,
            visual,
            strong: strong.iter().map(|n| path(n)).collect(),
            summary: VisualSummary::Ranked {
                name_matches: 1,
                indexed: 6,
                eligible: 6,
                strength: None,
                more: false,
                sorted: false,
                excluded: 0,
                not_applied: None,
            },
            entries,
            order: SortOrder::NameAsc,
        }
    }

    fn names(page: &BrowseResult) -> Vec<String> {
        page.paths
            .iter()
            .map(|p| p.file_stem().unwrap().to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn good_matches_show_sorted_and_show_more_adds_weaker_ones_closest_first() {
        use crate::visual_search::search::PAGE;
        let snapshot = snapshot(&["z", "a"]);
        // The name match and the strong matches, sorted by name.
        let first = snapshot.page(PAGE);
        assert_eq!(names(&first), ["a", "m-tower", "z"]);
        assert!(matches!(
            first.visual,
            Some(VisualSummary::Ranked {
                more: true,
                sorted: true,
                ..
            })
        ));
        let mut strong = first.strong.clone();
        strong.sort();
        assert_eq!(
            names(&BrowseResult {
                paths: strong,
                visual: None,
                strong: vec![]
            }),
            ["a", "m-tower", "z"]
        );
        // Show More: weaker matches below, closest first.
        let more = snapshot.page(2 * PAGE);
        assert_eq!(names(&more), ["a", "m-tower", "z", "b", "y", "x"]);
        assert!(matches!(
            more.visual,
            Some(VisualSummary::Ranked { more: false, .. })
        ));
    }

    #[test]
    fn all_strong_matches_form_a_fixed_sorted_block_before_weaker_pages() {
        use crate::visual_search::search::PAGE;
        let mut snapshot = snapshot(&[]);
        let template = snapshot.entries.values().next().unwrap().clone();
        let strong: Vec<_> = (0..2 * PAGE + 1)
            .rev()
            .map(|i| PathBuf::from(format!("/l/strong-{i:04}.jpg")))
            .collect();
        for path in &strong {
            let mut entry = template.clone();
            entry.path = path.clone();
            entry.filename = path.file_name().unwrap().to_string_lossy().into_owned();
            snapshot.entries.insert(path.clone(), entry);
        }
        snapshot.strong = strong.iter().cloned().collect();
        snapshot.visual.splice(0..0, strong);
        let first = snapshot.page(PAGE);
        assert_eq!(
            first.paths.len(),
            2 * PAGE + 2,
            "every strong match and the name match show immediately"
        );
        let mut expected = first.paths.clone();
        expected.sort();
        assert_eq!(first.paths, expected, "the entire good block is sorted");
        assert_eq!(
            first.strong.len(),
            first.paths.len(),
            "Strong Matches gets the entire good block"
        );
        let more = snapshot.page(2 * PAGE);
        assert_eq!(
            &more.paths[..first.paths.len()],
            first.paths.as_slice(),
            "Show More never moves the good block"
        );
        assert_eq!(
            &more.paths[first.paths.len()..],
            snapshot.visual[2 * PAGE + 1..].as_ref(),
            "only weaker matches are appended, closest first"
        );
        assert!(matches!(
            more.visual,
            Some(VisualSummary::Ranked { more: false, .. })
        ));
        snapshot.visual.truncate(2 * PAGE + 1);
        let only_good = snapshot.page(PAGE);
        assert_eq!(only_good.paths, first.paths);
        assert!(
            matches!(
                only_good.visual,
                Some(VisualSummary::Ranked { more: false, .. })
            ),
            "Show More is hidden when all matches are strong"
        );
    }

    #[test]
    fn without_strong_matches_the_closest_show_closest_first() {
        let page = snapshot(&[]).page(crate::visual_search::search::PAGE);
        assert_eq!(names(&page), ["m-tower", "z", "b", "y", "a", "x"]);
        assert!(matches!(
            page.visual,
            Some(VisualSummary::Ranked { sorted: false, .. })
        ));
    }

    #[test]
    fn later_queries_use_original_candidates_and_never_escape_scope() {
        let paths = vec![
            PathBuf::from("/folder/mountain.jpg"),
            PathBuf::from("/folder/sea.jpg"),
        ];
        assert_eq!(match_candidates(&paths, "mountain"), vec![paths[0].clone()]);
        assert_eq!(match_candidates(&paths, "sea"), vec![paths[1].clone()]);
        assert_eq!(match_candidates(&paths, ""), paths);
    }
}
