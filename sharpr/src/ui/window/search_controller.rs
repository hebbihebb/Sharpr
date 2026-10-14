//! Scoped browsing keeps an immutable candidate set while query results change.
use super::*;

type BrowseTask = Box<dyn FnOnce(&mut BrowseCaches) -> Result<BrowseResult, String> + Send>;

/// Rows for the filmstrip, plus how visual search went when it ran.
pub(super) struct BrowseResult {
    paths: Vec<PathBuf>,
    visual: Option<VisualSummary>,
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
    },
    /// Visual search couldn't run; file names were matched instead.
    Unavailable(String),
}

/// One visual search's complete ranking. "Show More" pages through it
/// without ranking again, so results already shown never move while
/// indexing adds vectors. A new query replaces it.
struct RankedSnapshot {
    /// The browse session and the search box's text it answers.
    key: (u64, String),
    by_name: Vec<PathBuf>,
    /// Visual results, closest first, without the file-name matches.
    visual: Vec<PathBuf>,
    summary: VisualSummary,
}

impl RankedSnapshot {
    fn page(&self, limit: usize) -> BrowseResult {
        let mut paths = self.by_name.clone();
        paths.extend(self.visual.iter().take(limit).cloned());
        let mut summary = self.summary.clone();
        if let VisualSummary::Ranked { more, .. } = &mut summary {
            *more = self.visual.len() > limit;
        }
        BrowseResult {
            paths,
            visual: Some(summary),
        }
    }
}

/// State the browse worker keeps between queries.
#[derive(Default)]
pub(super) struct BrowseCaches {
    pub dimensions: crate::model::browse::DimensionCache,
    /// Whole-library file list for one search session, keyed by session,
    /// library root and disabled folders. Walking the library tree on every
    /// keystroke grows with library size.
    library_paths: Option<(u64, PathBuf, Vec<PathBuf>, Vec<PathBuf>)>,
    ranked: Option<RankedSnapshot>,
}

impl BrowseCaches {
    fn library_paths(
        &mut self,
        session: u64,
        root: &Path,
        disabled: &[PathBuf],
        superseded: &dyn Fn() -> bool,
    ) -> Result<Vec<PathBuf>, String> {
        if let Some((cached_session, cached_root, cached_disabled, paths)) = &self.library_paths {
            if *cached_session == session && cached_root == root && cached_disabled == disabled {
                return Ok(paths.clone());
            }
        }
        let paths = crate::model::library::collect_library_image_paths_cancellable(
            Some(root.to_path_buf()),
            FolderMode::DrillDown,
            None,
            disabled,
            superseded,
        );
        // A walk cut short by a newer query is incomplete: never cache it.
        if superseded() {
            return Err("Search superseded".into());
        }
        self.library_paths = Some((
            session,
            root.to_path_buf(),
            disabled.to_vec(),
            paths.clone(),
        ));
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
}

#[derive(Clone, Default)]
pub(crate) struct BrowseOptions {
    pub dimensions: crate::model::browse::DimensionFilters,
    pub whole_library: bool,
    /// Chips taken out of the search box (decision D16).
    pub chips: crate::model::search_query::ChipFilters,
}
impl BrowseOptions {
    pub(super) fn has_filters(&self) -> bool {
        self.whole_library || !self.chips.is_empty() || !self.dimensions.is_empty()
    }
}

impl SharprWindow {
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
            st.library.restore_view(&entries, snapshot.folder);
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
    /// and into chips (decision D16).
    pub(super) fn take_completed_search_chips(&self, entry: &gtk4::SearchEntry) {
        let (chips, remaining) =
            crate::model::search_query::take_completed_chips(entry.text().as_str());
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
        let has_chips = !self.imp().browse_options.borrow().chips.is_empty();
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
            .is_some_and(|(status, _, _)| status.is_visible());
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
        container.set_visible(!chips.is_empty());
        self.sync_search_chip_bar();
        for chip in chips {
            let pill = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
            pill.add_css_class("linked");
            pill.set_halign(gtk4::Align::Start);
            let label = gtk4::Button::with_label(&chip.label());
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
            label.connect_clicked(move |_| {
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
        let Some((bar, label, more)) = self.imp().search_status.borrow().clone() else {
            return;
        };
        let text = summary.map(visual_summary_text);
        bar.set_visible(text.is_some());
        label.set_text(text.as_deref().unwrap_or_default());
        more.set_visible(matches!(
            summary,
            Some(VisualSummary::Ranked { more: true, .. })
        ));
        self.sync_search_chip_bar();
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
        let (candidates, root, disabled, scope, order, index) = {
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
                st.settings.active_library().map(|l| l.root.clone()),
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
                    let snapshot_key = (session, snapshot_query);
                    if extend {
                        if let Some(snapshot) = &cache.ranked {
                            if snapshot.key == snapshot_key {
                                return Ok(snapshot.page(visual_limit));
                            }
                        }
                    }
                    let candidates = if options.whole_library {
                        let root = root.clone().ok_or("No active library")?;
                        cache.library_paths(session, &root, &disabled, &superseded)?
                    } else {
                        candidates
                    };
                    let candidates = if options.whole_library {
                        filter_paths_for_library(candidates, root.as_deref(), &disabled)
                    } else {
                        candidates
                            .into_iter()
                            .filter(|p| !path_is_disabled(p, &disabled))
                            .collect()
                    };
                    // Words go to visual search when it can run; otherwise,
                    // and for file names, they match names.
                    let visual = (!search_terms(&text).is_empty())
                        .then_some(searcher.as_ref())
                        .flatten()
                        .map(|searcher| searcher.prepare(&text, &superseded));
                    if superseded() {
                        return Err("Search superseded".into());
                    }
                    let name_only = !matches!(visual, Some(Ok(_)));
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
                    let needs_dimensions = !options.dimensions.is_empty()
                        || chips.needs_dimensions()
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
                        if !options.dimensions.matches(dimensions)
                            || !chips.matches(&path, dimensions)
                        {
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
                    let prepared = match visual {
                        Some(Ok(prepared)) => prepared,
                        other => {
                            crate::model::library::sort_raw_entries(&mut raw, order);
                            return Ok(BrowseResult {
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
                    let fingerprints = index.cached_fingerprints().map_err(|e| e.to_string())?;
                    let mut by_name = Vec::new();
                    let mut candidates = Vec::with_capacity(raw.len());
                    for (entry, stamp) in raw.into_iter().zip(stamps) {
                        let fingerprint = fingerprints
                            .get(entry.path.to_string_lossy().as_ref())
                            .filter(|(size, mtime, _)| (*size, *mtime) == stamp)
                            .map(|(_, _, hash)| hash.clone());
                        candidates.push((entry.path.clone(), fingerprint));
                        if name_matches.contains(&entry.path) {
                            by_name.push(entry);
                        }
                    }
                    crate::model::library::sort_raw_entries(&mut by_name, order);
                    // Coverage and strength count every candidate; only the
                    // listing leaves the file-name matches out of the ranking.
                    let ranked = crate::visual_search::search::rank(&candidates, &prepared);
                    let visual: Vec<PathBuf> = ranked
                        .paths
                        .into_iter()
                        .filter(|path| !name_matches.contains(path))
                        .collect();
                    let snapshot = RankedSnapshot {
                        key: snapshot_key,
                        summary: VisualSummary::Ranked {
                            name_matches: by_name.len(),
                            indexed: ranked.indexed,
                            eligible: ranked.eligible,
                            strength: ranked.strength,
                            more: false,
                        },
                        by_name: by_name.into_iter().map(|r| r.path).collect(),
                        visual,
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
                Ok(BrowseResult { paths, visual }) => {
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
                    let ranked = matches!(visual, Some(VisualSummary::Ranked { .. }));
                    let selected = if ranked && !extend { None } else { selected };
                    if !extend {
                        win.imp().state.borrow_mut().selected_paths.clear();
                    }
                    load_virtual_async(&win.imp().state, &paths);
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

fn visual_summary_text(summary: &VisualSummary) -> String {
    use crate::visual_search::search::Strength;
    match summary {
        VisualSummary::Unavailable(reason) => {
            format!("Matching file names only. {}", reason.trim_end_matches('.'))
        }
        VisualSummary::Ranked {
            name_matches,
            indexed,
            eligible,
            strength,
            ..
        } => {
            let mut parts = Vec::new();
            if *name_matches > 0 {
                parts.push(format!("{name_matches} by file name first"));
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
            if indexed < eligible {
                parts.push(format!("{indexed} of {eligible} images indexed"));
            } else {
                parts.push(format!("all {eligible} images searched"));
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
            caches.library_paths(1, &root, &[], &current).unwrap().len(),
            1
        );
        std::fs::write(root.join("b.jpg"), b"b").unwrap();
        assert_eq!(
            caches.library_paths(1, &root, &[], &current).unwrap().len(),
            1,
            "same session reuses the walk"
        );
        assert!(caches.library_paths(2, &root, &[], &|| true).is_err());
        assert_eq!(
            caches.library_paths(2, &root, &[], &current).unwrap().len(),
            2,
            "a new session walks again; the cancelled walk was not kept"
        );

        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn show_more_extends_one_ranking_without_reordering() {
        let snapshot = RankedSnapshot {
            key: (1, "tower".into()),
            by_name: vec![PathBuf::from("/l/tower-notes.jpg")],
            visual: (0..5)
                .map(|i| PathBuf::from(format!("/l/{i}.jpg")))
                .collect(),
            summary: VisualSummary::Ranked {
                name_matches: 1,
                indexed: 6,
                eligible: 6,
                strength: None,
                more: false,
            },
        };
        let first = snapshot.page(2);
        let more = snapshot.page(4);
        assert_eq!(first.paths.len(), 3);
        assert_eq!(
            &more.paths[..3],
            &first.paths[..],
            "shown results never move"
        );
        assert!(matches!(
            first.visual,
            Some(VisualSummary::Ranked { more: true, .. })
        ));
        assert!(matches!(
            snapshot.page(5).visual,
            Some(VisualSummary::Ranked { more: false, .. })
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
