use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

use gdk4::Paintable;
use glib::prelude::*;
use gtk4::gio;
use gtk4::prelude::*;
use gtk4::subclass::prelude::*;

use crate::model::library::{SortField, SortOrder};
use crate::model::ImageEntry;
use crate::quality::QualityClass;
use crate::thumbnails::worker::WorkerRequest;
use crate::ui::window::{AppState, ViewScope};

type ImageSelectedCallback = Box<dyn Fn(u32) + 'static>;
type SearchChangedCallback = Box<dyn Fn(&str) + 'static>;
type SearchActivateCallback = Box<dyn Fn(&str) + 'static>;
type SearchDismissedCallback = Box<dyn Fn() + 'static>;
type TrashRequestedCallback = Box<dyn Fn(std::path::PathBuf) + 'static>;
type AddToCollectionRequestedCallback = Box<dyn Fn(Vec<PathBuf>) + 'static>;
type RemoveFromCollectionRequestedCallback = Box<dyn Fn(Vec<PathBuf>) + 'static>;
type SortOrderChangedCallback = Box<dyn Fn(SortOrder) + 'static>;
type QualityFilterChangedCallback = Box<dyn Fn(Option<QualityClass>) + 'static>;
type SaveSearchAsCollectionCallback = Box<dyn Fn(&str) + 'static>;

const ESTIMATED_ROW_HEIGHT: f64 = 220.0;
const BUFFER_ROWS: u32 = 2000;
const FALLBACK_VISIBLE_ROWS: u32 = 40;
const COLLECTION_COLOR_PALETTE: &[&str] = &[
    "#57e389", "#62a0ea", "#ff7800", "#f5c211", "#dc8add", "#5bc8af", "#e01b24", "#9141ac",
];

fn fallback_collection_color(collection_id: i64) -> &'static str {
    COLLECTION_COLOR_PALETTE[(collection_id as usize) % COLLECTION_COLOR_PALETTE.len()]
}

fn root_collection_id(id: i64, parent_by_id: &HashMap<i64, Option<i64>>) -> i64 {
    let mut root_id = id;
    let mut current = parent_by_id.get(&id).copied().flatten();
    while let Some(parent_id) = current {
        root_id = parent_id;
        current = parent_by_id.get(&parent_id).copied().flatten();
    }
    root_id
}

fn register_pill_color(color: &str) -> String {
    use std::sync::{LazyLock, Mutex};
    static REGISTERED: LazyLock<Mutex<std::collections::HashSet<String>>> =
        LazyLock::new(|| Mutex::new(std::collections::HashSet::new()));

    let hex = color.trim_start_matches('#');
    let key = hex.to_lowercase();
    let class_name = format!("filmstrip-pill-color-{key}");

    if let Ok(mut seen) = REGISTERED.lock() {
        if seen.insert(key) && hex.len() == 6 {
            if let (Ok(r), Ok(g), Ok(b)) = (
                u8::from_str_radix(&hex[0..2], 16),
                u8::from_str_radix(&hex[2..4], 16),
                u8::from_str_radix(&hex[4..6], 16),
            ) {
                let provider = gtk4::CssProvider::new();
                provider.load_from_string(&format!(
                    ".{class_name} {{ background-color: rgba({r},{g},{b},0.92); }}"
                ));
                if let Some(display) = gdk4::Display::default() {
                    gtk4::style_context_add_provider_for_display(
                        &display,
                        &provider,
                        gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
                    );
                }
            }
        }
    }
    class_name
}

fn collection_tag_color_map(
    collections: &[crate::library_index::Collection],
) -> HashMap<String, String> {
    let parent_by_id: HashMap<i64, Option<i64>> = collections
        .iter()
        .map(|collection| (collection.id, collection.parent_id))
        .collect();
    let collection_by_id: HashMap<i64, &crate::library_index::Collection> = collections
        .iter()
        .map(|collection| (collection.id, collection))
        .collect();
    let mut tag_to_color = HashMap::new();

    for collection in collections {
        let root_id = root_collection_id(collection.id, &parent_by_id);
        let resolved_color = collection_by_id
            .get(&root_id)
            .and_then(|root| root.color.clone())
            .unwrap_or_else(|| fallback_collection_color(root_id).to_string());

        tag_to_color.insert(collection.primary_tag.clone(), resolved_color.clone());
        for tag in &collection.extra_tags {
            tag_to_color.insert(tag.clone(), resolved_color.clone());
        }
    }

    tag_to_color
}

mod imp {
    use super::*;
    use async_channel::Sender;

    pub struct FilmstripPane {
        pub toolbar_view: libadwaita::ToolbarView,
        pub root_box: gtk4::Box,
        pub search_bar: gtk4::SearchBar,
        pub search_entry: gtk4::SearchEntry,
        pub save_search_btn: gtk4::Button,
        pub suggestions_popover: gtk4::Popover,
        pub suggestions_list: gtk4::ListBox,
        pub scroll: gtk4::ScrolledWindow,
        pub list_view: gtk4::ListView,
        pub selection_model: gtk4::SingleSelection,
        pub image_selected_cb: RefCell<Option<ImageSelectedCallback>>,
        pub search_changed_cb: RefCell<Option<SearchChangedCallback>>,
        pub search_activate_cb: RefCell<Option<SearchActivateCallback>>,
        pub search_dismissed_cb: RefCell<Option<SearchDismissedCallback>>,
        pub trash_requested_cb: RefCell<Option<TrashRequestedCallback>>,
        pub add_to_collection_requested_cb: RefCell<Option<AddToCollectionRequestedCallback>>,
        pub remove_from_collection_requested_cb:
            RefCell<Option<RemoveFromCollectionRequestedCallback>>,
        pub sort_order_changed_cb: RefCell<Option<SortOrderChangedCallback>>,
        pub quality_filter_changed_cb: RefCell<Option<QualityFilterChangedCallback>>,
        pub save_search_as_collection_cb: RefCell<Option<SaveSearchAsCollectionCallback>>,
        pub quality_radios: RefCell<Vec<(gtk4::CheckButton, Option<QualityClass>)>>,
        pub sort_field_radios: RefCell<Vec<(gtk4::CheckButton, SortField)>>,
        pub sort_direction_radios: RefCell<Vec<(gtk4::CheckButton, bool)>>,
        pub current_sort_order: Cell<SortOrder>,
        pub sort_btn: RefCell<Option<gtk4::MenuButton>>,
        pub state: RefCell<Option<Rc<RefCell<AppState>>>>,
        pub visible_thumbnail_tx: RefCell<Option<Sender<WorkerRequest>>>,
        pub preload_thumbnail_tx: RefCell<Option<Sender<WorkerRequest>>>,
        pub thumbnail_gen: RefCell<Option<Arc<std::sync::atomic::AtomicU64>>>,
        pub pending_thumbnails: RefCell<Option<Arc<Mutex<std::collections::HashSet<PathBuf>>>>>,
        pub pending_notify_count: std::sync::atomic::AtomicU32,
        pub tag_root_color: RefCell<HashMap<String, String>>,
        pub cached_tags: RefCell<Option<std::sync::Arc<crate::tags::TagDatabase>>>,
    }

    impl Default for FilmstripPane {
        fn default() -> Self {
            let selection_model = gtk4::SingleSelection::new(None::<gio::ListStore>);
            selection_model.set_can_unselect(false);

            let list_view =
                gtk4::ListView::new(Some(selection_model.clone()), None::<gtk4::ListItemFactory>);
            list_view.set_orientation(gtk4::Orientation::Vertical);
            list_view.add_css_class("navigation-sidebar");

            let search_bar = gtk4::SearchBar::new();
            let search_entry = gtk4::SearchEntry::new();
            let save_search_btn = {
                let b = gtk4::Button::from_icon_name("bookmark-new-symbolic");
                b.set_visible(false);
                b
            };
            let suggestions_list = gtk4::ListBox::new();
            let suggestions_popover = gtk4::Popover::new();

            Self {
                toolbar_view: libadwaita::ToolbarView::new(),
                root_box: gtk4::Box::new(gtk4::Orientation::Vertical, 0),
                search_bar,
                search_entry,
                save_search_btn,
                suggestions_popover,
                suggestions_list,
                scroll: gtk4::ScrolledWindow::new(),
                list_view,
                selection_model,
                image_selected_cb: RefCell::new(None),
                search_changed_cb: RefCell::new(None),
                search_activate_cb: RefCell::new(None),
                search_dismissed_cb: RefCell::new(None),
                trash_requested_cb: RefCell::new(None),
                add_to_collection_requested_cb: RefCell::new(None),
                remove_from_collection_requested_cb: RefCell::new(None),
                sort_order_changed_cb: RefCell::new(None),
                quality_filter_changed_cb: RefCell::new(None),
                save_search_as_collection_cb: RefCell::new(None),
                quality_radios: RefCell::new(Vec::new()),
                sort_field_radios: RefCell::new(Vec::new()),
                sort_direction_radios: RefCell::new(Vec::new()),
                current_sort_order: Cell::new(SortOrder::default()),
                sort_btn: RefCell::new(None),
                state: RefCell::new(None),
                visible_thumbnail_tx: RefCell::new(None),
                preload_thumbnail_tx: RefCell::new(None),
                thumbnail_gen: RefCell::new(None),
                pending_thumbnails: RefCell::new(None),
                pending_notify_count: std::sync::atomic::AtomicU32::new(0),
                tag_root_color: RefCell::new(HashMap::new()),
                cached_tags: RefCell::new(None),
            }
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for FilmstripPane {
        const NAME: &'static str = "SharprFilmstripPane";
        type Type = super::FilmstripPane;
        type ParentType = gtk4::Widget;

        fn class_init(klass: &mut Self::Class) {
            klass.set_layout_manager_type::<gtk4::BinLayout>();
        }
    }

    impl ObjectImpl for FilmstripPane {
        fn dispose(&self) {
            self.toolbar_view.unparent();
        }
    }

    impl WidgetImpl for FilmstripPane {}
}

glib::wrapper! {
    pub struct FilmstripPane(ObjectSubclass<imp::FilmstripPane>)
        @extends gtk4::Widget;
}

impl FilmstripPane {
    pub fn new(state: Rc<RefCell<AppState>>) -> Self {
        let widget: Self = glib::Object::new();
        *widget.imp().state.borrow_mut() = Some(state);
        widget.build_ui();
        widget
    }

    fn build_ui(&self) {
        let imp = self.imp();

        let header = libadwaita::HeaderBar::new();
        header.set_show_end_title_buttons(false);

        let sort_popover = gtk4::Popover::new();
        let sort_box = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        sort_box.add_css_class("menu");
        let default_sort = SortOrder::default();

        // Sort section.
        let name_row = gtk4::CheckButton::with_label("Name");
        let date_row = gtk4::CheckButton::with_label("Date Modified");
        date_row.set_group(Some(&name_row));
        let type_row = gtk4::CheckButton::with_label("Type");
        type_row.set_group(Some(&name_row));
        match default_sort.field() {
            SortField::Name => name_row.set_active(true),
            SortField::DateModified => date_row.set_active(true),
            SortField::FileType => type_row.set_active(true),
        }
        *imp.sort_field_radios.borrow_mut() = vec![
            (name_row.clone(), SortField::Name),
            (date_row.clone(), SortField::DateModified),
            (type_row.clone(), SortField::FileType),
        ];

        sort_box.append(&name_row);
        sort_box.append(&date_row);
        sort_box.append(&type_row);

        let direction_sep = gtk4::Separator::new(gtk4::Orientation::Horizontal);
        direction_sep.set_margin_top(4);
        direction_sep.set_margin_bottom(4);
        sort_box.append(&direction_sep);

        let ascending_row = gtk4::CheckButton::with_label("Ascending");
        let descending_row = gtk4::CheckButton::with_label("Descending");
        descending_row.set_group(Some(&ascending_row));
        if default_sort.descending() {
            descending_row.set_active(true);
        } else {
            ascending_row.set_active(true);
        }
        *imp.sort_direction_radios.borrow_mut() = vec![
            (ascending_row.clone(), false),
            (descending_row.clone(), true),
        ];
        sort_box.append(&ascending_row);
        sort_box.append(&descending_row);

        // Quality filter section.
        let sep = gtk4::Separator::new(gtk4::Orientation::Horizontal);
        sep.set_margin_top(4);
        sep.set_margin_bottom(4);
        sort_box.append(&sep);

        let all_quality = gtk4::CheckButton::with_label("All Quality");
        all_quality.set_active(true);
        sort_box.append(&all_quality);

        let mut quality_radios: Vec<(gtk4::CheckButton, Option<QualityClass>)> = Vec::new();
        quality_radios.push((all_quality.clone(), None));
        for &class in QualityClass::ALL.iter() {
            let radio = gtk4::CheckButton::with_label(class.label());
            radio.set_group(Some(&all_quality));
            sort_box.append(&radio);
            quality_radios.push((radio, Some(class)));
        }
        *imp.quality_radios.borrow_mut() = quality_radios;

        sort_popover.set_child(Some(&sort_box));

        let sort_btn = gtk4::MenuButton::new();
        sort_btn.set_icon_name("pan-down-symbolic");
        sort_btn.set_tooltip_text(Some("Sort and filter"));
        sort_btn.set_popover(Some(&sort_popover));
        header.pack_end(&sort_btn);
        *imp.sort_btn.borrow_mut() = Some(sort_btn.clone());
        imp.toolbar_view.add_top_bar(&header);

        let w = self.downgrade();
        name_row.connect_toggled(move |btn| {
            if btn.is_active() {
                if let Some(f) = w.upgrade() {
                    let sort_order = SortOrder::from_parts(
                        SortField::Name,
                        f.imp().current_sort_order.get().descending(),
                    );
                    f.imp().current_sort_order.set(sort_order);
                    if let Some(cb) = f.imp().sort_order_changed_cb.borrow().as_ref() {
                        cb(sort_order);
                    }
                }
            }
        });

        let w = self.downgrade();
        date_row.connect_toggled(move |btn| {
            if btn.is_active() {
                if let Some(f) = w.upgrade() {
                    let sort_order = SortOrder::from_parts(
                        SortField::DateModified,
                        f.imp().current_sort_order.get().descending(),
                    );
                    f.imp().current_sort_order.set(sort_order);
                    if let Some(cb) = f.imp().sort_order_changed_cb.borrow().as_ref() {
                        cb(sort_order);
                    }
                }
            }
        });

        let w = self.downgrade();
        type_row.connect_toggled(move |btn| {
            if btn.is_active() {
                if let Some(f) = w.upgrade() {
                    let sort_order = SortOrder::from_parts(
                        SortField::FileType,
                        f.imp().current_sort_order.get().descending(),
                    );
                    f.imp().current_sort_order.set(sort_order);
                    if let Some(cb) = f.imp().sort_order_changed_cb.borrow().as_ref() {
                        cb(sort_order);
                    }
                }
            }
        });

        let w = self.downgrade();
        ascending_row.connect_toggled(move |btn| {
            if btn.is_active() {
                if let Some(f) = w.upgrade() {
                    let sort_order =
                        SortOrder::from_parts(f.imp().current_sort_order.get().field(), false);
                    f.imp().current_sort_order.set(sort_order);
                    if let Some(cb) = f.imp().sort_order_changed_cb.borrow().as_ref() {
                        cb(sort_order);
                    }
                }
            }
        });

        let w = self.downgrade();
        descending_row.connect_toggled(move |btn| {
            if btn.is_active() {
                if let Some(f) = w.upgrade() {
                    let sort_order =
                        SortOrder::from_parts(f.imp().current_sort_order.get().field(), true);
                    f.imp().current_sort_order.set(sort_order);
                    if let Some(cb) = f.imp().sort_order_changed_cb.borrow().as_ref() {
                        cb(sort_order);
                    }
                }
            }
        });

        for (radio, class) in imp.quality_radios.borrow().clone() {
            let w = self.downgrade();
            radio.connect_toggled(move |btn| {
                if btn.is_active() {
                    if let Some(f) = w.upgrade() {
                        if let Some(cb) = f.imp().quality_filter_changed_cb.borrow().as_ref() {
                            cb(class);
                        }
                    }
                }
            });
        }

        self.install_css();

        imp.search_entry.set_placeholder_text(Some("Search tags…"));
        let search_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 4);
        search_row.append(&imp.search_entry);
        imp.save_search_btn.add_css_class("flat");
        imp.save_search_btn.set_valign(gtk4::Align::Center);
        imp.save_search_btn
            .set_tooltip_text(Some("Save search as Collection"));
        search_row.append(&imp.save_search_btn);
        imp.search_bar.set_child(Some(&search_row));
        imp.search_bar.set_show_close_button(true);
        imp.search_bar.connect_entry(&imp.search_entry);

        imp.suggestions_list.add_css_class("boxed-list");
        imp.suggestions_list
            .set_selection_mode(gtk4::SelectionMode::None);
        imp.suggestions_popover.set_has_arrow(false);
        imp.suggestions_popover.set_autohide(true);
        imp.suggestions_popover
            .set_position(gtk4::PositionType::Bottom);
        imp.suggestions_popover
            .set_child(Some(&imp.suggestions_list));
        imp.suggestions_popover.set_parent(&imp.search_entry);

        let factory = gtk4::SignalListItemFactory::new();

        let widget_weak_setup = self.downgrade();
        let widget_weak_bind = self.downgrade();
        factory.connect_setup(move |_, obj| {
            let list_item = obj
                .downcast_ref::<gtk4::ListItem>()
                .expect("factory object must be ListItem");

            let item_box = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
            item_box.set_margin_top(4);
            item_box.set_margin_bottom(4);
            item_box.set_margin_start(4);
            item_box.set_margin_end(4);

            let overlay = gtk4::Overlay::new();
            let picture = gtk4::Picture::new();
            picture.set_content_fit(gtk4::ContentFit::Cover);
            picture.set_size_request(-1, 160);
            picture.add_css_class("thumbnail-frame");

            let index_label = gtk4::Label::new(None);
            index_label.set_halign(gtk4::Align::Start);
            index_label.set_valign(gtk4::Align::End);
            index_label.set_xalign(0.0);
            index_label.set_margin_start(5);
            index_label.set_margin_bottom(5);
            index_label.add_css_class("filmstrip-index-label");

            let badge = gtk4::DrawingArea::new();
            badge.set_content_width(12);
            badge.set_content_height(12);
            badge.set_halign(gtk4::Align::Start);
            badge.set_valign(gtk4::Align::End);
            badge.set_margin_start(5);
            badge.set_margin_bottom(5);
            badge.set_visible(false);

            overlay.set_child(Some(&picture));
            overlay.add_overlay(&index_label);
            overlay.add_overlay(&badge);

            let filename_label = gtk4::Label::new(None);
            filename_label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
            filename_label.set_max_width_chars(18);
            filename_label.add_css_class("caption");

            // Append content children before parenting the popover — in GTK4,
            // set_parent() inserts at the front of the child list so doing it
            // first would make the popover the first_child(), breaking traversal.
            item_box.append(&overlay);
            item_box.append(&filename_label);

            let popover = gtk4::Popover::new();
            popover.set_has_arrow(true);
            popover.set_autohide(true);
            popover.set_position(gtk4::PositionType::Bottom);
            popover.set_parent(&item_box);

            // Store widget refs by key so bind/unbind don't rely on child ordering.
            unsafe {
                list_item.set_data("fs-picture", picture.clone());
                list_item.set_data("fs-index-label", index_label.clone());
                list_item.set_data("fs-badge", badge.clone());
                list_item.set_data("fs-filename-label", filename_label.clone());
                list_item.set_data("fs-item-box", item_box.clone());
            }

            let popover_box = gtk4::Box::new(gtk4::Orientation::Vertical, 6);
            popover_box.set_margin_top(6);
            popover_box.set_margin_bottom(6);
            popover_box.set_margin_start(6);
            popover_box.set_margin_end(6);

            let open_button = gtk4::Button::with_label("Open in Default Viewer");
            let reveal_button = gtk4::Button::with_label("Show in File Manager");
            let copy_button = gtk4::Button::with_label("Copy to Clipboard");
            open_button.set_halign(gtk4::Align::Fill);
            reveal_button.set_halign(gtk4::Align::Fill);
            copy_button.set_halign(gtk4::Align::Fill);
            popover_box.append(&open_button);
            popover_box.append(&reveal_button);
            popover_box.append(&copy_button);

            let separator = gtk4::Separator::new(gtk4::Orientation::Horizontal);
            popover_box.append(&separator);

            let add_to_collection_button = gtk4::Button::with_label("Add to Collection\u{2026}");
            add_to_collection_button.set_halign(gtk4::Align::Fill);
            popover_box.append(&add_to_collection_button);

            let remove_from_collection_button = gtk4::Button::with_label("Remove from Collection");
            remove_from_collection_button.set_halign(gtk4::Align::Fill);
            popover_box.append(&remove_from_collection_button);

            let separator = gtk4::Separator::new(gtk4::Orientation::Horizontal);
            popover_box.append(&separator);

            let trash_button = gtk4::Button::with_label("Move to Trash");
            trash_button.set_halign(gtk4::Align::Fill);
            trash_button.add_css_class("destructive-action");
            popover_box.append(&trash_button);

            popover.set_child(Some(&popover_box));

            let gesture_right = gtk4::GestureClick::new();
            gesture_right.set_button(3);
            item_box.add_controller(gesture_right.clone());

            // Ctrl+Click: toggle item in multi-selection.
            let gesture_ctrl = gtk4::GestureClick::new();
            gesture_ctrl.set_button(1);
            item_box.add_controller(gesture_ctrl.clone());

            let gesture_primary = gtk4::GestureClick::new();
            gesture_primary.set_button(1);
            item_box.add_controller(gesture_primary.clone());

            let list_item_weak = list_item.downgrade();
            let widget_weak_ctrl = widget_weak_setup.clone();
            gesture_ctrl.connect_released(move |gesture, _, _, _| {
                let modifiers = gesture.current_event_state();
                if !modifiers.contains(gdk4::ModifierType::CONTROL_MASK) {
                    return;
                }
                gesture.set_state(gtk4::EventSequenceState::Claimed);
                let Some(list_item) = list_item_weak.upgrade() else {
                    return;
                };
                let Some(path) = list_item_path(&list_item) else {
                    return;
                };
                let Some(widget) = widget_weak_ctrl.upgrade() else {
                    return;
                };
                widget.toggle_action_selection_path(&path);
            });

            let list_item_weak = list_item.downgrade();
            let widget_weak_primary = widget_weak_setup.clone();
            gesture_primary.connect_released(move |gesture, _, _, _| {
                let modifiers = gesture.current_event_state();
                if modifiers.contains(gdk4::ModifierType::CONTROL_MASK) {
                    return;
                }
                let Some(list_item) = list_item_weak.upgrade() else {
                    return;
                };
                let Some(path) = list_item_path(&list_item) else {
                    return;
                };
                let Some(widget) = widget_weak_primary.upgrade() else {
                    return;
                };
                widget.set_action_selection_to_path(&path);
            });

            // Drag source: drag this item (or all multi-selected items) to a sidebar collection.
            let drag_source = gtk4::DragSource::new();
            drag_source.set_actions(gdk4::DragAction::COPY);
            item_box.add_controller(drag_source.clone());

            let list_item_weak = list_item.downgrade();
            let widget_weak_drag = widget_weak_setup.clone();
            drag_source.connect_prepare(move |_, _, _| {
                let list_item = list_item_weak.upgrade()?;
                let dragged_path = list_item_path(&list_item)?;
                let widget = widget_weak_drag.upgrade()?;
                let paths = collection_action_paths(&widget, &dragged_path);
                let paths_str: String = paths
                    .iter()
                    .map(|p| p.to_string_lossy().into_owned())
                    .collect::<Vec<_>>()
                    .join("\n");
                Some(gdk4::ContentProvider::for_value(&paths_str.to_value()))
            });

            // Double-click is handled by ListView::activate at the pane level.

            let list_item_weak = list_item.downgrade();
            open_button.connect_clicked(move |_| {
                let Some(list_item) = list_item_weak.upgrade() else {
                    return;
                };
                if let Some(path) = list_item_path(&list_item) {
                    launch_default_for_path(&path);
                }
            });

            let list_item_weak = list_item.downgrade();
            reveal_button.connect_clicked(move |_| {
                let Some(list_item) = list_item_weak.upgrade() else {
                    return;
                };
                if let Some(path) = list_item_path(&list_item) {
                    reveal_in_file_manager(&path);
                }
            });

            let list_item_weak = list_item.downgrade();
            let item_box_weak = item_box.downgrade();
            let popover_weak_copy = popover.downgrade();
            copy_button.connect_clicked(move |_| {
                let Some(list_item) = list_item_weak.upgrade() else {
                    return;
                };
                let Some(item_box) = item_box_weak.upgrade() else {
                    return;
                };
                if let Some(p) = popover_weak_copy.upgrade() {
                    p.popdown();
                }
                if let Some(path) = list_item_path(&list_item) {
                    copy_image_to_clipboard(&path, item_box.upcast_ref::<gtk4::Widget>());
                }
            });

            let list_item_weak = list_item.downgrade();
            let widget_weak_trash = widget_weak_setup.clone();
            let popover_weak_trash = popover.downgrade();
            trash_button.connect_clicked(move |_| {
                let Some(list_item) = list_item_weak.upgrade() else {
                    return;
                };
                let Some(widget) = widget_weak_trash.upgrade() else {
                    return;
                };
                if let Some(p) = popover_weak_trash.upgrade() {
                    p.popdown();
                }
                if let Some(path) = list_item_path(&list_item) {
                    widget.emit_trash_requested(path);
                }
            });

            let list_item_weak = list_item.downgrade();
            let widget_weak_add = widget_weak_setup.clone();
            let popover_weak_add = popover.downgrade();
            add_to_collection_button.connect_clicked(move |_| {
                let Some(list_item) = list_item_weak.upgrade() else {
                    return;
                };
                let Some(widget) = widget_weak_add.upgrade() else {
                    return;
                };
                let Some(clicked_path) = list_item_path(&list_item) else {
                    return;
                };
                if let Some(p) = popover_weak_add.upgrade() {
                    p.popdown();
                }
                let paths = collection_action_paths(&widget, &clicked_path);
                widget.emit_add_to_collection_requested(paths);
            });

            let list_item_weak = list_item.downgrade();
            let widget_weak_remove = widget_weak_setup.clone();
            let popover_weak_remove = popover.downgrade();
            remove_from_collection_button.connect_clicked(move |_| {
                let Some(list_item) = list_item_weak.upgrade() else {
                    return;
                };
                let Some(widget) = widget_weak_remove.upgrade() else {
                    return;
                };
                let Some(clicked_path) = list_item_path(&list_item) else {
                    return;
                };
                if let Some(p) = popover_weak_remove.upgrade() {
                    p.popdown();
                }
                let paths = collection_action_paths(&widget, &clicked_path);
                widget.emit_remove_from_collection_requested(paths);
            });

            let add_btn_weak = add_to_collection_button.downgrade();
            let remove_btn_weak = remove_from_collection_button.downgrade();
            let widget_weak_gesture = widget_weak_setup.clone();
            let list_item_weak = list_item.downgrade();
            let popover_weak = popover.downgrade();
            gesture_right.connect_released(move |_, _, _, _| {
                let Some(list_item) = list_item_weak.upgrade() else {
                    return;
                };
                if list_item.item().is_none() {
                    return;
                }

                if let Some(widget) = widget_weak_gesture.upgrade() {
                    let (has_library_index, in_collection) = widget
                        .imp()
                        .state
                        .borrow()
                        .as_ref()
                        .map(|s| {
                            let s = s.borrow();
                            (
                                s.library_index.is_some(),
                                matches!(s.scope, ViewScope::Collection(_)),
                            )
                        })
                        .unwrap_or((false, false));
                    if let Some(btn) = add_btn_weak.upgrade() {
                        btn.set_sensitive(has_library_index);
                    }
                    if let Some(btn) = remove_btn_weak.upgrade() {
                        btn.set_visible(in_collection);
                        btn.set_sensitive(in_collection);
                    }
                }

                if let Some(popover) = popover_weak.upgrade() {
                    popover.popup();
                }
            });

            list_item.set_child(Some(&item_box));
        });

        factory.connect_bind(move |_, obj| {
            let list_item = obj
                .downcast_ref::<gtk4::ListItem>()
                .expect("factory object must be ListItem");

            let entry: ImageEntry = match list_item.item().and_downcast::<ImageEntry>() {
                Some(entry) => entry,
                None => return,
            };
            let (picture, index_label, badge, filename_label, item_box) = unsafe {
                let picture = list_item
                    .data::<gtk4::Picture>("fs-picture")
                    .map(|p| p.as_ref().clone());
                let index_label = list_item
                    .data::<gtk4::Label>("fs-index-label")
                    .map(|p| p.as_ref().clone());
                let badge = list_item
                    .data::<gtk4::DrawingArea>("fs-badge")
                    .map(|p| p.as_ref().clone());
                let filename_label = list_item
                    .data::<gtk4::Label>("fs-filename-label")
                    .map(|p| p.as_ref().clone());
                let item_box = list_item
                    .data::<gtk4::Box>("fs-item-box")
                    .map(|p| p.as_ref().clone());
                match (picture, index_label, badge, filename_label) {
                    (Some(p), Some(i), Some(b), Some(f)) => (p, i, b, f, item_box),
                    _ => return,
                }
            };

            // Restore multi-select visual state when items are recycled.
            if let Some(ib) = item_box {
                unsafe {
                    ib.set_data("fs-path", entry.path());
                }
                let is_selected = widget_weak_bind
                    .upgrade()
                    .and_then(|w| {
                        w.imp()
                            .state
                            .borrow()
                            .as_ref()
                            .and_then(|s| s.try_borrow().ok())
                            .map(|s| {
                                s.selected_paths.len() > 1
                                    && s.selected_paths.contains(&entry.path())
                            })
                    })
                    .unwrap_or(false);
                if is_selected {
                    ib.add_css_class("multi-selected");
                } else {
                    ib.remove_css_class("multi-selected");
                }
            }

            filename_label.set_text(&entry.filename());
            index_label.set_text(&(list_item.position() + 1).to_string());

            // Always hide the separate badge dot; collection color lives in the pill instead.
            badge.set_visible(false);

            let badge_color = widget_weak_bind.upgrade().and_then(|w| {
                let tags = w.imp().cached_tags.borrow().clone()?;
                let path = entry.path();
                let tag_root_color = w.imp().tag_root_color.borrow();
                tags.tags_for_path(&path)
                    .into_iter()
                    .find_map(|tag| tag_root_color.get(&tag).cloned())
            });

            // Remove any previously applied color class from this recycled label.
            let old_class: Option<String> = unsafe { list_item.steal_data("fs-pill-class") };
            if let Some(ref cls) = old_class {
                index_label.remove_css_class(cls);
            }

            if let Some(color) = badge_color {
                let class_name = register_pill_color(&color);
                index_label.add_css_class(&class_name);
                unsafe {
                    list_item.set_data("fs-pill-class", class_name);
                }
            }

            match entry.thumbnail() {
                Some(ref texture) => picture.set_paintable(Some(texture.upcast_ref::<Paintable>())),
                None => picture.set_paintable(None::<&Paintable>),
            }

            let picture_weak = picture.downgrade();
            let entry_clone = entry.clone();
            let handler_id = entry.connect_notify_local(Some("thumbnail"), move |_, _| {
                if let Some(picture) = picture_weak.upgrade() {
                    match entry_clone.thumbnail() {
                        Some(ref texture) => {
                            picture.set_paintable(Some(texture.upcast_ref::<Paintable>()))
                        }
                        None => picture.set_paintable(None::<&Paintable>),
                    }
                }
            });

            unsafe {
                list_item.set_data("thumbnail-notify-id", handler_id);
            }
        });

        factory.connect_unbind(|_, obj| {
            let list_item = obj
                .downcast_ref::<gtk4::ListItem>()
                .expect("factory object must be ListItem");
            if let Some(entry) = list_item.item().and_downcast::<ImageEntry>() {
                let handler_id =
                    unsafe { list_item.steal_data::<glib::SignalHandlerId>("thumbnail-notify-id") };
                if let Some(handler_id) = handler_id {
                    entry.disconnect(handler_id);
                }
            }
            unsafe {
                if let Some(picture) = list_item
                    .data::<gtk4::Picture>("fs-picture")
                    .map(|p| p.as_ref().clone())
                {
                    picture.set_paintable(None::<&Paintable>);
                }
                if let Some(label) = list_item
                    .data::<gtk4::Label>("fs-index-label")
                    .map(|p| p.as_ref().clone())
                {
                    label.set_text("");
                }
                if let Some(badge) = list_item
                    .data::<gtk4::DrawingArea>("fs-badge")
                    .map(|p| p.as_ref().clone())
                {
                    badge.set_visible(false);
                }
                if let Some(label) = list_item
                    .data::<gtk4::Label>("fs-filename-label")
                    .map(|p| p.as_ref().clone())
                {
                    label.set_text("");
                }
            }
        });

        imp.list_view.set_factory(Some(&factory));
        // ListView::activate fires on double-click or Enter — use it for open-in-viewer.
        // Single-click still changes selection via SingleSelection; this is double-click only.
        imp.list_view.set_single_click_activate(false);

        imp.scroll
            .set_policy(gtk4::PolicyType::Never, gtk4::PolicyType::Automatic);
        imp.scroll.set_vexpand(true);
        imp.scroll.set_hexpand(true);
        imp.scroll.set_child(Some(&imp.list_view));

        imp.root_box.append(&imp.scroll);
        imp.toolbar_view.set_content(Some(&imp.root_box));
        imp.toolbar_view.set_parent(self);

        // Throttle scroll-driven scheduling: fire at most once per 80 ms to
        // avoid saturating the main thread during fast scrolls.
        let scroll_pending = std::rc::Rc::new(std::cell::Cell::new(false));
        let widget_weak = self.downgrade();
        imp.scroll.vadjustment().connect_value_changed(move |_| {
            if scroll_pending.get() {
                return;
            }
            scroll_pending.set(true);
            let pending_c = scroll_pending.clone();
            let widget_weak_c = widget_weak.clone();
            glib::timeout_add_local(std::time::Duration::from_millis(80), move || {
                pending_c.set(false);
                if let Some(widget) = widget_weak_c.upgrade() {
                    widget.schedule_visible_thumbnails();
                }
                glib::ControlFlow::Break
            });
        });

        let widget_weak = self.downgrade();
        self.connect_map(move |_| {
            if let Some(widget) = widget_weak.upgrade() {
                widget.schedule_visible_thumbnails();
            }
        });

        let widget_weak = self.downgrade();
        imp.selection_model
            .connect_selected_notify(move |selection| {
                let index = selection.selected();
                if index == gtk4::INVALID_LIST_POSITION {
                    return;
                }
                if let Some(widget) = widget_weak.upgrade() {
                    widget.emit_image_selected(index);
                }
            });

        let widget_weak = self.downgrade();
        imp.search_entry.connect_search_changed(move |entry| {
            if let Some(widget) = widget_weak.upgrade() {
                widget.emit_search_changed(entry.text().as_ref());
            }
        });

        let btn_weak = imp.save_search_btn.downgrade();
        imp.search_entry.connect_search_changed(move |entry| {
            if let Some(btn) = btn_weak.upgrade() {
                btn.set_visible(!entry.text().is_empty());
            }
        });

        let widget_weak = self.downgrade();
        imp.search_entry.connect_activate(move |entry| {
            if let Some(widget) = widget_weak.upgrade() {
                widget.emit_search_activate(entry.text().as_ref());
            }
        });

        let widget_weak = self.downgrade();
        imp.search_bar
            .connect_search_mode_enabled_notify(move |search_bar| {
                if !search_bar.is_search_mode() {
                    if let Some(widget) = widget_weak.upgrade() {
                        widget.show_autocomplete(vec![]);
                        widget.emit_search_dismissed();
                    }
                }
            });

        let btn_weak = imp.save_search_btn.downgrade();
        imp.search_bar
            .connect_search_mode_enabled_notify(move |bar| {
                if let Some(btn) = btn_weak.upgrade() {
                    if !bar.is_search_mode() {
                        btn.set_visible(false);
                    }
                }
            });

        let widget_weak = self.downgrade();
        imp.save_search_btn.connect_clicked(move |_| {
            let Some(widget) = widget_weak.upgrade() else {
                return;
            };
            let query = widget.imp().search_entry.text().to_string();
            widget.emit_save_search_as_collection(&query);
        });

        let search_entry = imp.search_entry.clone();
        imp.suggestions_list.connect_row_activated(move |_, row| {
            let Some(child) = row.child() else { return };
            let Ok(label) = child.downcast::<gtk4::Label>() else {
                return;
            };
            search_entry.set_text(label.text().as_ref());
            search_entry.activate();
        });
    }

    fn install_css(&self) {
        let provider = gtk4::CssProvider::new();
        provider.load_from_string(
            "
            .filmstrip-index-label {
                font-size: 11px;
                font-weight: 500;
                padding: 2px 6px;
                background-color: rgba(0, 0, 0, 0.55);
                color: white;
                border-radius: 10px;
            }
            .multi-selected {
                outline: 2px solid @accent_color;
                outline-offset: -2px;
                border-radius: 4px;
            }
            ",
        );
        if let Some(display) = gdk4::Display::default() {
            gtk4::style_context_add_provider_for_display(
                &display,
                &provider,
                gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
            );
        }
    }

    pub fn refresh(&self) {
        let imp = self.imp();
        let state_opt = imp.state.borrow();
        let Some(ref state_rc) = *state_opt else {
            return;
        };
        let store = state_rc.borrow().library.store.clone();
        imp.selection_model.set_model(None::<&gio::ListStore>);
        imp.selection_model.set_model(Some(&store));
        self.schedule_visible_thumbnails();
        let widget = self.downgrade();
        glib::idle_add_local(move || {
            let Some(filmstrip) = widget.upgrade() else {
                return glib::ControlFlow::Break;
            };
            let page_size = filmstrip.imp().scroll.vadjustment().page_size();
            filmstrip.schedule_visible_thumbnails();
            if page_size > 0.0 {
                glib::ControlFlow::Break
            } else {
                glib::ControlFlow::Continue
            }
        });
    }

    pub fn refresh_virtual(&self) {
        self.refresh();
    }

    pub fn refresh_collection_colors(&self, collections: &[crate::library_index::Collection]) {
        *self.imp().tag_root_color.borrow_mut() = collection_tag_color_map(collections);
        self.imp().list_view.queue_draw();
    }

    pub fn set_cached_tags(&self, tags: std::sync::Arc<crate::tags::TagDatabase>) {
        *self.imp().cached_tags.borrow_mut() = Some(tags);
    }

    pub fn set_thumbnail_sender(
        &self,
        visible_tx: async_channel::Sender<WorkerRequest>,
        preload_tx: async_channel::Sender<WorkerRequest>,
        gen: Arc<std::sync::atomic::AtomicU64>,
        pending_set: Arc<Mutex<std::collections::HashSet<PathBuf>>>,
    ) {
        *self.imp().visible_thumbnail_tx.borrow_mut() = Some(visible_tx);
        *self.imp().preload_thumbnail_tx.borrow_mut() = Some(preload_tx);
        *self.imp().thumbnail_gen.borrow_mut() = Some(gen);
        *self.imp().pending_thumbnails.borrow_mut() = Some(pending_set);
        self.schedule_visible_thumbnails();
    }

    pub fn mark_thumbnail_ready(&self, path: &Path) {
        let imp = self.imp();
        if let Some(pending) = imp.pending_thumbnails.borrow().as_ref() {
            if let Ok(mut pending) = pending.lock() {
                pending.remove(path);
            }
        }

        // Throttle UI updates: increment counter and only spawn the idle
        // task if it's the first pending notification.
        if imp.pending_notify_count.fetch_add(1, Ordering::Relaxed) == 0 {
            let widget_weak = self.downgrade();
            glib::idle_add_local(move || {
                if let Some(widget) = widget_weak.upgrade() {
                    widget
                        .imp()
                        .pending_notify_count
                        .store(0, Ordering::Relaxed);
                    widget.schedule_visible_thumbnails();
                }
                glib::ControlFlow::Break
            });
        }
    }

    pub fn schedule_visible_thumbnails(&self) {
        let imp = self.imp();
        let Some(visible_tx) = imp.visible_thumbnail_tx.borrow().as_ref().cloned() else {
            return;
        };
        let Some(preload_tx) = imp.preload_thumbnail_tx.borrow().as_ref().cloned() else {
            return;
        };
        let Some(pending_set) = imp.pending_thumbnails.borrow().as_ref().cloned() else {
            return;
        };
        let gen = imp
            .thumbnail_gen
            .borrow()
            .as_ref()
            .map_or(0, |a| a.load(Ordering::Relaxed));
        let Some(state_rc) = imp.state.borrow().as_ref().cloned() else {
            return;
        };

        let adjustment = imp.scroll.vadjustment();
        let visible_start = (adjustment.value() / ESTIMATED_ROW_HEIGHT).floor().max(0.0) as u32;
        let page_size = adjustment.page_size();
        let visible_rows = if page_size > 0.0 {
            (page_size / ESTIMATED_ROW_HEIGHT).ceil() as u32
        } else {
            0
        };

        let mut visible_range_start = visible_start;
        let mut visible_range_end = visible_start.saturating_add(visible_rows);
        let mut preload_range_start = visible_start.saturating_sub(BUFFER_ROWS);
        let mut preload_range_end = visible_start
            .saturating_add(visible_rows)
            .saturating_add(BUFFER_ROWS);

        let image_count = state_rc.borrow().library.image_count();
        let mut visible_capped_end = visible_range_end.min(image_count);
        let mut preload_capped_end = preload_range_end.min(image_count);

        if visible_rows == 0 || visible_capped_end <= visible_range_start {
            visible_range_start = 0;
            visible_range_end = FALLBACK_VISIBLE_ROWS;
            preload_range_start = 0;
            preload_range_end = FALLBACK_VISIBLE_ROWS.saturating_add(BUFFER_ROWS);
            visible_capped_end = visible_range_end.min(image_count);
            preload_capped_end = preload_range_end.min(image_count);
        }

        let mut visible_worker_paths: Vec<PathBuf> = Vec::new();
        let mut preload_worker_paths: Vec<PathBuf> = Vec::new();

        {
            let state = state_rc.borrow();
            for index in preload_range_start..preload_capped_end {
                let Some(entry) = state.library.entry_at(index) else {
                    continue;
                };
                if entry.thumbnail().is_some() {
                    continue;
                }
                let path = entry.path();
                // Workers check the disk cache first (fast path in generate_thumbnail),
                // so we just enqueue and let them handle it off the main thread.
                let is_visible =
                    index >= visible_range_start && index < visible_capped_end && visible_rows > 0;
                if is_visible {
                    visible_worker_paths.push(path);
                } else {
                    preload_worker_paths.push(path);
                }
            }
        }

        let visible_candidates = visible_worker_paths.len();
        let preload_candidates = preload_worker_paths.len();
        let mut visible_enqueued = 0usize;
        let mut preload_enqueued = 0usize;

        for path in visible_worker_paths {
            let should_enqueue = {
                let Ok(mut pending) = pending_set.lock() else {
                    continue;
                };
                pending.insert(path.clone())
            };
            if !should_enqueue {
                continue;
            }
            if visible_tx
                .try_send(WorkerRequest::Thumbnail {
                    path: path.clone(),
                    gen,
                })
                .is_err()
            {
                if let Ok(mut pending) = pending_set.lock() {
                    pending.remove(&path);
                }
                break;
            }
            visible_enqueued += 1;
        }

        for path in preload_worker_paths {
            let should_enqueue = {
                let Ok(mut pending) = pending_set.lock() else {
                    continue;
                };
                pending.insert(path.clone())
            };
            if !should_enqueue {
                continue;
            }
            if preload_tx
                .try_send(WorkerRequest::Thumbnail {
                    path: path.clone(),
                    gen,
                })
                .is_err()
            {
                if let Ok(mut pending) = pending_set.lock() {
                    pending.remove(&path);
                }
                break;
            }
            preload_enqueued += 1;
        }

        if visible_candidates > 0 || preload_candidates > 0 {
            crate::bench_event!(
                "filmstrip.thumbnail_schedule",
                serde_json::json!({
                    "image_count": image_count,
                    "visible_start": visible_range_start,
                    "visible_end": visible_capped_end,
                    "preload_start": preload_range_start,
                    "preload_end": preload_capped_end,
                    "visible_candidates": visible_candidates,
                    "preload_candidates": preload_candidates,
                    "visible_enqueued": visible_enqueued,
                    "preload_enqueued": preload_enqueued,
                    "gen": gen,
                }),
            );
        }
    }

    pub fn connect_item_activated<F: Fn(u32) + 'static>(&self, f: F) {
        self.imp()
            .list_view
            .connect_activate(move |_, position| f(position));
    }

    pub fn navigate_to(&self, index: u32) {
        self.imp().list_view.scroll_to(
            index,
            gtk4::ListScrollFlags::SELECT | gtk4::ListScrollFlags::FOCUS,
            None,
        );
    }

    pub fn scroll_to_index(&self, index: u32) {
        let adjustment = self.imp().scroll.vadjustment();
        // If the layout hasn't happened yet (upper not yet computed), defer until
        // the scroll window knows its content size.
        if adjustment.upper() <= adjustment.page_size() + 1.0 {
            let widget_weak = self.downgrade();
            glib::idle_add_local(move || {
                let Some(widget) = widget_weak.upgrade() else {
                    return glib::ControlFlow::Break;
                };
                let adj = widget.imp().scroll.vadjustment();
                if adj.upper() <= adj.page_size() + 1.0 {
                    return glib::ControlFlow::Continue;
                }
                let upper_bound = (adj.upper() - adj.page_size()).max(0.0);
                let offset = (index as f64 * ESTIMATED_ROW_HEIGHT).min(upper_bound);
                adj.set_value(offset);
                glib::ControlFlow::Break
            });
            return;
        }
        let upper_bound = (adjustment.upper() - adjustment.page_size()).max(0.0);
        let offset = (index as f64 * ESTIMATED_ROW_HEIGHT).min(upper_bound);
        adjustment.set_value(offset);
    }

    pub fn select_index(&self, index: u32) {
        self.imp().selection_model.set_selected(index);
    }

    pub fn activate_search(&self) {
        let imp = self.imp();
        imp.search_bar.set_search_mode(true);
        imp.search_entry.grab_focus();
    }

    pub fn is_search_active(&self) -> bool {
        self.imp().search_bar.is_search_mode()
    }

    pub fn deactivate_search(&self) {
        let imp = self.imp();
        imp.search_bar.set_search_mode(false);
        imp.search_entry.set_text("");
        self.show_autocomplete(vec![]);
    }

    pub fn set_search_capture_widget<W: IsA<gtk4::Widget>>(&self, widget: &W) {
        self.imp().search_bar.set_key_capture_widget(Some(widget));
    }

    pub fn connect_search_changed<F: Fn(&str) + 'static>(&self, f: F) {
        *self.imp().search_changed_cb.borrow_mut() = Some(Box::new(f));
    }

    pub fn connect_search_activate<F: Fn(&str) + 'static>(&self, f: F) {
        *self.imp().search_activate_cb.borrow_mut() = Some(Box::new(f));
    }

    pub fn connect_search_dismissed<F: Fn() + 'static>(&self, f: F) {
        *self.imp().search_dismissed_cb.borrow_mut() = Some(Box::new(f));
    }

    pub fn connect_trash_requested<F: Fn(std::path::PathBuf) + 'static>(&self, f: F) {
        *self.imp().trash_requested_cb.borrow_mut() = Some(Box::new(f));
    }

    pub fn set_sort_order_changed_cb<F: Fn(SortOrder) + 'static>(&self, cb: F) {
        *self.imp().sort_order_changed_cb.borrow_mut() = Some(Box::new(cb));
    }

    pub fn connect_quality_filter_changed<F: Fn(Option<QualityClass>) + 'static>(&self, cb: F) {
        *self.imp().quality_filter_changed_cb.borrow_mut() = Some(Box::new(cb));
    }

    pub fn connect_save_search_as_collection<F: Fn(&str) + 'static>(&self, f: F) {
        *self.imp().save_search_as_collection_cb.borrow_mut() = Some(Box::new(f));
    }

    /// Resets the quality filter radio to "All Quality" without emitting the callback.
    pub fn reset_quality_filter(&self) {
        if let Some((all_radio, _)) = self.imp().quality_radios.borrow().first() {
            all_radio.set_active(true);
        }
    }

    pub fn show_autocomplete(&self, suggestions: Vec<String>) {
        let imp = self.imp();
        while let Some(child) = imp.suggestions_list.first_child() {
            imp.suggestions_list.remove(&child);
        }

        if suggestions.is_empty() {
            imp.suggestions_popover.popdown();
            return;
        }

        for suggestion in suggestions {
            let row = gtk4::ListBoxRow::new();
            let label = gtk4::Label::new(Some(&suggestion));
            label.set_halign(gtk4::Align::Start);
            label.set_margin_top(6);
            label.set_margin_bottom(6);
            label.set_margin_start(10);
            label.set_margin_end(10);
            row.set_child(Some(&label));
            imp.suggestions_list.append(&row);
        }

        imp.suggestions_popover.popup();
    }

    pub fn connect_image_selected<F: Fn(u32) + 'static>(&self, f: F) {
        *self.imp().image_selected_cb.borrow_mut() = Some(Box::new(f));
    }

    fn emit_image_selected(&self, index: u32) {
        if let Some(cb) = self.imp().image_selected_cb.borrow().as_ref() {
            cb(index);
        }
    }

    fn emit_search_changed(&self, text: &str) {
        if let Some(cb) = self.imp().search_changed_cb.borrow().as_ref() {
            cb(text);
        }
    }

    pub fn emit_search_activate(&self, text: &str) {
        if let Some(cb) = self.imp().search_activate_cb.borrow().as_ref() {
            cb(text);
        }
    }

    fn emit_save_search_as_collection(&self, query: &str) {
        if let Some(cb) = self.imp().save_search_as_collection_cb.borrow().as_ref() {
            cb(query);
        }
    }

    fn emit_search_dismissed(&self) {
        if let Some(cb) = self.imp().search_dismissed_cb.borrow().as_ref() {
            cb();
        }
    }

    fn emit_trash_requested(&self, path: std::path::PathBuf) {
        if let Some(cb) = self.imp().trash_requested_cb.borrow().as_ref() {
            cb(path);
        }
    }

    pub fn connect_add_to_collection_requested<F: Fn(Vec<PathBuf>) + 'static>(&self, f: F) {
        *self.imp().add_to_collection_requested_cb.borrow_mut() = Some(Box::new(f));
    }

    pub fn connect_remove_from_collection_requested<F: Fn(Vec<PathBuf>) + 'static>(&self, f: F) {
        *self.imp().remove_from_collection_requested_cb.borrow_mut() = Some(Box::new(f));
    }

    fn emit_add_to_collection_requested(&self, paths: Vec<PathBuf>) {
        if let Some(cb) = self.imp().add_to_collection_requested_cb.borrow().as_ref() {
            cb(paths);
        }
    }

    fn emit_remove_from_collection_requested(&self, paths: Vec<PathBuf>) {
        if let Some(cb) = self
            .imp()
            .remove_from_collection_requested_cb
            .borrow()
            .as_ref()
        {
            cb(paths);
        }
    }

    pub fn clear_multi_selection(&self) {
        if let Some(state) = self.imp().state.borrow().as_ref() {
            state.borrow_mut().selected_paths.clear();
        }
        self.refresh_multi_selection_visuals();
    }

    pub fn set_action_selection_to_path(&self, path: &Path) {
        if let Some(state) = self.imp().state.borrow().as_ref() {
            let mut state = state.borrow_mut();
            state.selected_paths.clear();
            if state.library.entry_for_path(path).is_some() {
                state.selected_paths.insert(path.to_path_buf());
            }
        }
        self.refresh_multi_selection_visuals();
    }

    pub fn toggle_action_selection_path(&self, path: &Path) {
        if let Some(state) = self.imp().state.borrow().as_ref() {
            let mut state = state.borrow_mut();
            if state.selected_paths.is_empty() {
                if let Some(current) = state.library.selected_entry().map(|entry| entry.path()) {
                    state.selected_paths.insert(current);
                }
            }
            if state.selected_paths.contains(path) {
                state.selected_paths.remove(path);
            } else if state.library.entry_for_path(path).is_some() {
                state.selected_paths.insert(path.to_path_buf());
            }
        }
        self.refresh_multi_selection_visuals();
    }

    pub fn refresh_multi_selection_visuals(&self) {
        let selected_paths = self
            .imp()
            .state
            .borrow()
            .as_ref()
            .and_then(|state| state.try_borrow().ok())
            .map(|state| state.selected_paths.clone())
            .unwrap_or_default();
        let show_multi = selected_paths.len() > 1;
        sync_visible_selection_classes(
            self.imp().list_view.upcast_ref::<gtk4::Widget>(),
            &selected_paths,
            show_multi,
        );
    }
}

/// Returns the effective set of paths for a collection action (add or remove).
/// If `selected_paths` is non-empty and includes `clicked_path`, returns all of them.
/// Otherwise returns just `clicked_path`.
fn collection_action_paths(widget: &FilmstripPane, clicked_path: &Path) -> Vec<PathBuf> {
    widget
        .imp()
        .state
        .borrow()
        .as_ref()
        .and_then(|s| {
            let s = s.borrow();
            if !s.selected_paths.is_empty() && s.selected_paths.contains(clicked_path) {
                Some(s.selected_paths.iter().cloned().collect())
            } else {
                None
            }
        })
        .unwrap_or_else(|| vec![clicked_path.to_path_buf()])
}

fn list_item_path(list_item: &gtk4::ListItem) -> Option<PathBuf> {
    list_item
        .item()
        .and_downcast::<ImageEntry>()
        .map(|entry| entry.path())
}

fn sync_visible_selection_classes(
    widget: &gtk4::Widget,
    selected_paths: &std::collections::HashSet<PathBuf>,
    show_multi: bool,
) {
    if let Some(item_box) = widget.downcast_ref::<gtk4::Box>() {
        let is_selected = unsafe {
            item_box
                .data::<PathBuf>("fs-path")
                .map(|path| show_multi && selected_paths.contains(path.as_ref()))
                .unwrap_or(false)
        };
        if is_selected {
            item_box.add_css_class("multi-selected");
        } else {
            item_box.remove_css_class("multi-selected");
        }
    }

    let mut child = widget.first_child();
    while let Some(current) = child {
        sync_visible_selection_classes(&current, selected_paths, show_multi);
        child = current.next_sibling();
    }
}

fn launch_default_for_path(path: &Path) {
    let uri = gio::File::for_path(path).uri();
    let _ = gio::AppInfo::launch_default_for_uri(&uri, gio::AppLaunchContext::NONE);
}

fn copy_image_to_clipboard(path: &Path, widget: &gtk4::Widget) {
    let file = gio::File::for_path(path);
    if let Ok(texture) = gdk4::Texture::from_file(&file) {
        widget.clipboard().set_texture(&texture);
    }
}

fn reveal_in_file_manager(path: &Path) {
    // Use org.freedesktop.FileManager1.ShowItems to highlight the specific file.
    // Nautilus, Thunar, Dolphin, and most GNOME/KDE file managers implement this.
    let uri = gio::File::for_path(path).uri().to_string();
    let parent = path.parent().map(|p| p.to_path_buf());
    std::thread::spawn(move || {
        let ok = std::process::Command::new("dbus-send")
            .args([
                "--session",
                "--dest=org.freedesktop.FileManager1",
                "--type=method_call",
                "/org/freedesktop/FileManager1",
                "org.freedesktop.FileManager1.ShowItems",
                &format!("array:string:{}", uri),
                "string:",
            ])
            .status()
            .is_ok_and(|s| s.success());
        if !ok {
            // Fallback: open the parent directory.
            if let Some(parent_path) = parent {
                let _ = std::process::Command::new("xdg-open")
                    .arg(&parent_path)
                    .spawn();
            }
        }
    });
}
