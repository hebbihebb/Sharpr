//! Scoped browsing keeps an immutable candidate set while query results change.
use super::*;

type BrowseTask = Box<dyn FnOnce(&mut BrowseCaches) -> Result<Vec<PathBuf>, String> + Send>;

/// State the browse worker keeps between queries.
#[derive(Default)]
pub(super) struct BrowseCaches {
    pub dimensions: crate::model::browse::DimensionCache,
    /// Whole-library file list for one search session, keyed by session,
    /// library root and disabled folders. Walking the library tree on every
    /// keystroke grows with library size.
    library_paths: Option<(u64, PathBuf, Vec<PathBuf>, Vec<PathBuf>)>,
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
    response: async_channel::Sender<Result<Vec<PathBuf>, String>>,
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
        response: async_channel::Sender<Result<Vec<PathBuf>, String>>,
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
    pub tags: Vec<String>,
    pub whole_library: bool,
    /// Chips taken out of the search box (decision D16).
    pub chips: crate::model::search_query::ChipFilters,
}
impl BrowseOptions {
    pub(super) fn has_filters(&self) -> bool {
        self.whole_library
            || !self.chips.is_empty()
            || !self.dimensions.is_empty()
            || !self.tags.is_empty()
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
        if let Some(bar) = self.imp().search_chip_bar.borrow().as_ref() {
            bar.set_reveal_child(has_chips || has_suggestions);
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

    pub(super) fn run_browse_query(&self, query: String, sort_only: bool) {
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
            self.restore_browse_view();
            return;
        }
        self.capture_browse_view();
        let (candidates, tags, root, disabled, scope, order) = {
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
                st.tags.clone(),
                st.settings.active_library().map(|l| l.root.clone()),
                st.disabled_folders.clone(),
                st.scope.clone(),
                st.sort_order,
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
        let (tx, rx) = async_channel::bounded(1);
        let expected_query = query.clone();
        let canceled = self.imp().browse_generation.clone();
        self.imp().browse_worker.submit(
            generation,
            tx,
            Box::new(move |cache| {
                let result = (|| -> Result<Vec<PathBuf>, String> {
                    let superseded =
                        || canceled.load(std::sync::atomic::Ordering::Relaxed) != generation;
                    if superseded() {
                        return Err("Search superseded".into());
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
                    let tag_paths = tags
                        .as_ref()
                        .map(|db| db.search_paths_checked(&text))
                        .transpose()?;
                    let mut paths =
                        match_candidates(&candidates, tag_paths.as_deref().unwrap_or(&[]), &text);
                    if !options.tags.is_empty() {
                        let matching: std::collections::HashSet<_> = tags
                            .as_ref()
                            .ok_or("Tag database is unavailable")?
                            .paths_for_all_tags_checked(&options.tags)?
                            .into_iter()
                            .collect();
                        paths.retain(|p| matching.contains(p));
                    }
                    let needs_dimensions = !options.dimensions.is_empty()
                        || chips.needs_dimensions()
                        || matches!(
                            order.field(),
                            crate::model::library::SortField::Aspect
                                | crate::model::library::SortField::WideScreen
                        );
                    let mut raw = Vec::with_capacity(paths.len());
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
                    crate::model::library::sort_raw_entries(&mut raw, order);
                    Ok(raw.into_iter().map(|r| r.path).collect())
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
                Ok(paths) => {
                    let selected = win
                        .imp()
                        .state
                        .borrow()
                        .library
                        .selected_index
                        .and_then(|i| win.imp().state.borrow().library.entry_at(i))
                        .map(|e| e.path());
                    win.bump_thumbnail_generation("search.apply");
                    win.imp().state.borrow_mut().selected_paths.clear();
                    load_virtual_async(&win.imp().state, &paths);
                    win.present_browse_paths(selected.as_deref());
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

fn match_candidates(candidates: &[PathBuf], tag_paths: &[PathBuf], query: &str) -> Vec<PathBuf> {
    let terms = search_terms(query);
    let tags: std::collections::HashSet<_> = tag_paths.iter().collect();
    candidates
        .iter()
        .filter(|p| {
            tags.contains(p)
                || p.file_name().and_then(|n| n.to_str()).is_some_and(|name| {
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
    fn later_queries_use_original_candidates_and_never_escape_scope() {
        let paths = vec![
            PathBuf::from("/folder/mountain.jpg"),
            PathBuf::from("/folder/sea.jpg"),
        ];
        let tagged = vec![PathBuf::from("/other/mountain.jpg")];
        assert_eq!(
            match_candidates(&paths, &tagged, "mountain"),
            vec![paths[0].clone()]
        );
        assert_eq!(match_candidates(&paths, &[], "sea"), vec![paths[1].clone()]);
        assert_eq!(match_candidates(&paths, &[], ""), paths);
    }
}
