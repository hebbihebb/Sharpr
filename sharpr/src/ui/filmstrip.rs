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
use crate::thumbnails::worker::WorkerRequest;
use crate::ui::state::{AppState, ViewScope};

type ImageSelectedCallback = Box<dyn Fn(u32) + 'static>;
type SearchChangedCallback = Box<dyn Fn(&str) + 'static>;
type SearchActivateCallback = Box<dyn Fn(&str) + 'static>;
type SearchDismissedCallback = Box<dyn Fn() + 'static>;
type TrashRequestedCallback = Box<dyn Fn(Vec<PathBuf>) + 'static>;
type AddToCollectionRequestedCallback = Box<dyn Fn(Vec<PathBuf>) + 'static>;
type CopyRequestedCallback = Box<dyn Fn(CopyKind, Vec<PathBuf>) + 'static>;
type InfoRequestedCallback = Box<dyn Fn(PathBuf) + 'static>;

/// Copies offered on an image's right-click menu (decision D21).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CopyKind {
    /// Crop the clicked image.
    Crop,
    /// Export the clicked image, or the selection, as WebP (Phase 6).
    Export,
}
type ActionSelectionChangedCallback = Box<dyn Fn(usize) + 'static>;
type RemoveFromCollectionRequestedCallback = Box<dyn Fn(Vec<PathBuf>) + 'static>;
type CopyToFolderRequestedCallback = Box<dyn Fn(Vec<PathBuf>) + 'static>;
type MoveToFolderRequestedCallback = Box<dyn Fn(Vec<PathBuf>) + 'static>;
type RenameRequestedCallback = Box<dyn Fn(PathBuf) + 'static>;
type DuplicateRequestedCallback = Box<dyn Fn(PathBuf) + 'static>;

type SortOrderChangedCallback = Box<dyn Fn(SortOrder) + 'static>;
type SaveSearchAsCollectionCallback = Box<dyn Fn(&str) + 'static>;

pub(crate) struct MenuContext {
    pub(crate) clicked_path: PathBuf,
    pub(crate) action_paths: Vec<PathBuf>,
}

fn menu_item(label: &str, action: &str, accel: Option<&str>) -> gio::MenuItem {
    let item = gio::MenuItem::new(Some(label), Some(action));
    let icon = match action {
        "filmstrip.add-to-collection" | "filmstrip.add-last-bucket" => {
            Some("sharpr-add-bucket-symbolic")
        }
        "filmstrip.remove-from-collection" => Some("sharpr-bucket-symbolic"),
        "win.compare" => Some("sharpr-compare-symbolic"),
        "filmstrip.crop-copy" => Some("sharpr-crop-symbolic"),
        "filmstrip.export-webp" => Some("sharpr-webp-symbolic"),
        _ => None,
    };
    if let Some(icon) = icon {
        item.set_icon(&gio::ThemedIcon::new(icon));
    }

    if let Some(accel) = accel {
        item.set_attribute_value("accel", Some(&accel.to_variant()));
    }
    item
}

fn build_context_menu_model(
    in_collection: bool,
    is_multi: bool,
    last_bucket: Option<&str>,
) -> gio::Menu {
    let menu = gio::Menu::new();

    let sec1 = gio::Menu::new();
    if !is_multi {
        sec1.append_item(&menu_item("Info", "filmstrip.info", Some("<Control>i")));
    }
    sec1.append_item(&menu_item(
        "Open in Default Viewer",
        "filmstrip.open",
        Some("Return"),
    ));
    sec1.append_item(&menu_item("Show in File Manager", "filmstrip.reveal", None));
    sec1.append_item(&menu_item(
        "Copy to Clipboard",
        "filmstrip.copy-to-clipboard",
        Some("<Control>c"),
    ));
    menu.append_section(None, &sec1);

    let image = gio::Menu::new();
    if is_multi {
        image.append_item(&menu_item(
            "Compare Selected Images",
            "win.compare",
            Some("<Control><Shift>c"),
        ));
        image.append_item(&menu_item(
            "Survey Selected Images",
            "win.survey",
            Some("<Control><Shift>s"),
        ));
    } else {
        image.append_item(&menu_item("Crop Copy\u{2026}", "filmstrip.crop-copy", None));
    }
    image.append_item(&menu_item(
        "Export as WebP\u{2026}",
        "filmstrip.export-webp",
        Some("<Control>e"),
    ));
    menu.append_section(None, &image);

    let sec2 = gio::Menu::new();
    sec2.append_item(&menu_item(
        "Add to Bucket\u{2026}",
        "filmstrip.add-to-collection",
        None,
    ));
    let repeat_label = last_bucket
        .map(|name| format!("Add to {name}"))
        .unwrap_or_else(|| "Add to Last Bucket".into());
    sec2.append_item(&menu_item(
        &repeat_label,
        "filmstrip.add-last-bucket",
        Some("<Control><Shift>b"),
    ));
    if in_collection {
        sec2.append_item(&menu_item(
            "Remove from Bucket",
            "filmstrip.remove-from-collection",
            None,
        ));
    }
    let file_actions = gio::Menu::new();
    file_actions.append_item(&menu_item(
        "Copy to Folder\u{2026}",
        "filmstrip.copy-to-folder",
        None,
    ));
    file_actions.append_item(&menu_item(
        "Move to Folder\u{2026}",
        "filmstrip.move-to-folder",
        None,
    ));
    if !is_multi {
        file_actions.append_item(&menu_item("Rename\u{2026}", "filmstrip.rename", Some("F2")));
        file_actions.append_item(&menu_item(
            "Duplicate\u{2026}",
            "filmstrip.duplicate",
            Some("<Control>d"),
        ));
    }
    sec2.append_submenu(Some("File Actions"), &file_actions);
    menu.append_section(None, &sec2);

    let sec4 = gio::Menu::new();
    sec4.append_item(&menu_item(
        "Move to Trash",
        "filmstrip.trash",
        Some("Delete"),
    ));
    menu.append_section(None, &sec4);

    menu
}

const ESTIMATED_ROW_HEIGHT: f64 = 220.0;
const BUFFER_ROWS: u32 = 100;
const FALLBACK_VISIBLE_ROWS: u32 = 12;
const MAX_PRELOAD_ENQUEUE_PER_PASS: usize = 1024;
const THUMBNAIL_RESCHEDULE_DEBOUNCE_MS: u64 = 30;
const PRELOAD_CHAIN_DELAY_MS: u64 = 10;
fn register_pill_color(color: &str) -> String {
    super::look::color_class(color, true)
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
        pub content_stack: gtk4::Stack,
        pub empty_status: libadwaita::StatusPage,
        pub selection_model: gtk4::SingleSelection,
        pub image_selected_cb: RefCell<Option<ImageSelectedCallback>>,
        pub search_changed_cb: RefCell<Option<SearchChangedCallback>>,
        pub search_activate_cb: RefCell<Option<SearchActivateCallback>>,
        pub search_dismissed_cb: RefCell<Option<SearchDismissedCallback>>,
        pub trash_requested_cb: RefCell<Option<TrashRequestedCallback>>,
        pub add_to_collection_requested_cb: RefCell<Option<AddToCollectionRequestedCallback>>,
        pub copy_requested_cb: RefCell<Option<CopyRequestedCallback>>,
        pub info_requested_cb: RefCell<Option<InfoRequestedCallback>>,
        pub action_selection_changed_cb: RefCell<Option<ActionSelectionChangedCallback>>,
        /// Where a Shift-click range starts: the last plain or Ctrl-click.
        pub selection_anchor: RefCell<Option<PathBuf>>,
        pub remove_from_collection_requested_cb:
            RefCell<Option<RemoveFromCollectionRequestedCallback>>,
        pub copy_to_folder_requested_cb: RefCell<Option<CopyToFolderRequestedCallback>>,
        pub move_to_folder_requested_cb: RefCell<Option<MoveToFolderRequestedCallback>>,
        pub rename_requested_cb: RefCell<Option<RenameRequestedCallback>>,
        pub duplicate_requested_cb: RefCell<Option<DuplicateRequestedCallback>>,
        #[allow(private_interfaces)]
        pub menu_ctx: RefCell<Option<MenuContext>>,
        pub context_actions: gio::SimpleActionGroup,
        pub clipboard_decoder: RefCell<Option<crate::image_pipeline::clipboard::Decoder>>,
        pub sort_order_changed_cb: RefCell<Option<SortOrderChangedCallback>>,
        pub save_search_as_collection_cb: RefCell<Option<SaveSearchAsCollectionCallback>>,
        pub current_sort_order: Cell<SortOrder>,
        pub sort_btn: RefCell<Option<gtk4::MenuButton>>,
        pub header_bar: RefCell<Option<libadwaita::HeaderBar>>,
        pub state: RefCell<Option<Rc<RefCell<AppState>>>>,
        pub visible_thumbnail_tx: RefCell<Option<Sender<WorkerRequest>>>,
        pub preload_thumbnail_tx: RefCell<Option<Sender<WorkerRequest>>>,
        pub thumbnail_gen: RefCell<Option<Arc<std::sync::atomic::AtomicU64>>>,
        pub pending_thumbnails: RefCell<Option<Arc<Mutex<std::collections::HashSet<PathBuf>>>>>,
        pub pending_notify_count: std::sync::atomic::AtomicU32,
        pub refresh_schedule_pending: Cell<bool>,
        /// The active library's buckets, for badge colours.
        pub bucket_collections: RefCell<Vec<crate::library_index::Collection>>,
        /// Badge colour per path, built off the GTK thread from bucket
        /// membership so binding a row needs no database query.
        pub path_colors: RefCell<Option<HashMap<PathBuf, String>>>,
        pub path_colors_generation: Cell<u64>,
        pub cached_index: RefCell<Option<Arc<crate::library_index::LibraryIndex>>>,
        pub last_scroll_value: Cell<f64>,
        pub last_buffer_scan_time: Cell<Option<std::time::Instant>>,
        pub action_group: RefCell<Option<gio::SimpleActionGroup>>,
    }

    impl Default for FilmstripPane {
        fn default() -> Self {
            let selection_model = gtk4::SingleSelection::new(None::<gio::ListStore>);
            selection_model.set_can_unselect(false);

            let list_view =
                gtk4::ListView::new(Some(selection_model.clone()), None::<gtk4::ListItemFactory>);
            list_view.set_orientation(gtk4::Orientation::Vertical);
            list_view.add_css_class("navigation-sidebar");
            // Ctrl+A belongs to the window, which selects every image shown.
            // The list's own select-all can't work on a single selection and
            // would swallow the key.
            list_view.action_set_enabled("list.select-all", false);

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
                content_stack: gtk4::Stack::new(),
                empty_status: libadwaita::StatusPage::builder()
                    .icon_name("image-x-generic-symbolic")
                    .title("No images in this folder")
                    .build(),
                selection_model,
                image_selected_cb: RefCell::new(None),
                search_changed_cb: RefCell::new(None),
                search_activate_cb: RefCell::new(None),
                search_dismissed_cb: RefCell::new(None),
                trash_requested_cb: RefCell::new(None),
                add_to_collection_requested_cb: RefCell::new(None),
                copy_requested_cb: RefCell::new(None),
                info_requested_cb: RefCell::new(None),
                action_selection_changed_cb: RefCell::new(None),
                selection_anchor: RefCell::new(None),
                remove_from_collection_requested_cb: RefCell::new(None),
                copy_to_folder_requested_cb: RefCell::new(None),
                move_to_folder_requested_cb: RefCell::new(None),
                rename_requested_cb: RefCell::new(None),
                duplicate_requested_cb: RefCell::new(None),
                menu_ctx: RefCell::new(None),
                context_actions: gio::SimpleActionGroup::new(),
                clipboard_decoder: RefCell::new(None),
                sort_order_changed_cb: RefCell::new(None),
                save_search_as_collection_cb: RefCell::new(None),
                current_sort_order: Cell::new(SortOrder::default()),
                sort_btn: RefCell::new(None),
                header_bar: RefCell::new(None),
                state: RefCell::new(None),
                visible_thumbnail_tx: RefCell::new(None),
                preload_thumbnail_tx: RefCell::new(None),
                thumbnail_gen: RefCell::new(None),
                pending_thumbnails: RefCell::new(None),
                pending_notify_count: std::sync::atomic::AtomicU32::new(0),
                refresh_schedule_pending: Cell::new(false),
                bucket_collections: RefCell::new(Vec::new()),
                path_colors: RefCell::new(None),
                path_colors_generation: Cell::new(0),
                cached_index: RefCell::new(None),
                last_scroll_value: Cell::new(0.0),
                last_buffer_scan_time: Cell::new(None),
                action_group: RefCell::new(None),
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
            self.clipboard_decoder.borrow_mut().take();
            self.toolbar_view.unparent();
        }
    }

    impl WidgetImpl for FilmstripPane {}
}

glib::wrapper! {
    pub struct FilmstripPane(ObjectSubclass<imp::FilmstripPane>)
        @extends gtk4::Widget,
                 @implements gtk4::Accessible, gtk4::Buildable, gtk4::ConstraintTarget;
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

        let sort_btn = gtk4::MenuButton::new();
        sort_btn.set_icon_name("pan-down-symbolic");
        sort_btn.set_tooltip_text(Some("Sort and filter"));
        header.pack_end(&sort_btn);
        *imp.sort_btn.borrow_mut() = Some(sort_btn.clone());
        *imp.header_bar.borrow_mut() = Some(header.clone());
        imp.toolbar_view.add_top_bar(&header);

        let group = gio::SimpleActionGroup::new();

        // Sort Field Action
        let widget_weak = self.downgrade();
        let sort_field = gio::SimpleAction::new_stateful(
            "sort-field",
            Some(glib::VariantTy::new("s").unwrap()),
            &"date".to_variant(),
        );
        let sort_group = group.downgrade();
        sort_field.connect_activate(move |action, param| {
            if let Some(field_str) = param.and_then(|p| p.get::<String>()) {
                action.set_state(param.expect("Action should have a parameter"));
                if let Some(f) = widget_weak.upgrade() {
                    let field = match field_str.as_str() {
                        "name" => SortField::Name,
                        "date" => SortField::DateModified,
                        "type" => SortField::FileType,
                        "aspect" => SortField::Aspect,
                        "widescreen" => SortField::WideScreen,
                        _ => SortField::Name,
                    };
                    let sort_order = SortOrder::from_parts(
                        field,
                        if field == SortField::WideScreen {
                            false
                        } else {
                            f.imp().current_sort_order.get().descending()
                        },
                    );
                    f.imp().current_sort_order.set(sort_order);
                    if let Some(group) = sort_group.upgrade() {
                        if let Some(direction) = group.lookup_action("sort-direction") {
                            if let Ok(direction) = direction.downcast::<gio::SimpleAction>() {
                                direction.set_state(&sort_order.descending().to_variant());
                            }
                        }
                    }
                    if let Some(cb) = f.imp().sort_order_changed_cb.borrow().as_ref() {
                        cb(sort_order);
                    }
                }
            }
        });
        group.add_action(&sort_field);

        // Sort Direction Action
        let widget_weak = self.downgrade();
        let sort_direction = gio::SimpleAction::new_stateful(
            "sort-direction",
            Some(glib::VariantTy::new("b").unwrap()),
            &true.to_variant(),
        );
        sort_direction.connect_activate(move |action, param| {
            if let Some(descending) = param.and_then(|p| p.get::<bool>()) {
                action.set_state(param.expect("Action should have a parameter"));
                if let Some(f) = widget_weak.upgrade() {
                    let sort_order =
                        SortOrder::from_parts(f.imp().current_sort_order.get().field(), descending);
                    f.imp().current_sort_order.set(sort_order);
                    if let Some(cb) = f.imp().sort_order_changed_cb.borrow().as_ref() {
                        cb(sort_order);
                    }
                }
            }
        });
        group.add_action(&sort_direction);

        self.insert_action_group("filmstrip-sort", Some(&group));
        *imp.action_group.borrow_mut() = Some(group);

        let menu = gio::Menu::new();
        let sort_section = gio::Menu::new();
        sort_section.append_item(&gio::MenuItem::new(
            Some("Name"),
            Some("filmstrip-sort.sort-field::name"),
        ));
        sort_section.append_item(&gio::MenuItem::new(
            Some("Date Modified"),
            Some("filmstrip-sort.sort-field::date"),
        ));
        sort_section.append_item(&gio::MenuItem::new(
            Some("Type"),
            Some("filmstrip-sort.sort-field::type"),
        ));
        sort_section.append(
            Some("Aspect Ratio"),
            Some("filmstrip-sort.sort-field::aspect"),
        );
        sort_section.append(
            Some("Closest to 16:9"),
            Some("filmstrip-sort.sort-field::widescreen"),
        );
        menu.append_section(Some("Sort By"), &sort_section);

        let direction_section = gio::Menu::new();
        for (label, descending) in [("Ascending", false), ("Descending", true)] {
            let item = gio::MenuItem::new(Some(label), None);
            item.set_action_and_target_value(
                Some("filmstrip-sort.sort-direction"),
                Some(&descending.to_variant()),
            );
            direction_section.append_item(&item);
        }
        menu.append_section(None, &direction_section);

        sort_btn.set_menu_model(Some(&menu));

        imp.search_entry.set_placeholder_text(Some("Search tags…"));
        let search_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 4);
        search_row.append(&imp.search_entry);
        imp.save_search_btn.add_css_class("flat");
        imp.save_search_btn.set_valign(gtk4::Align::Center);
        imp.save_search_btn
            .set_tooltip_text(Some("Save search as Bucket"));
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
            crate::ui::look::thumbnail_feedback(&picture);
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

            // Context menu: a GtkPopoverMenu whose model is swapped before each popup.
            // Actions are dispatched through the pane-level "filmstrip" action group
            // (registered on FilmstripPane in setup_context_menu_actions, which walks
            // the widget hierarchy and finds it from the popover's parent chain).
            let popover = gtk4::PopoverMenu::from_model(None::<&gio::Menu>);
            popover.set_has_arrow(true);
            popover.set_autohide(true);
            popover.set_position(gtk4::PositionType::Bottom);
            popover.set_parent(&item_box);
            // Shift+F10 finds a row's menu through this (decision D21).
            unsafe {
                item_box.set_data("fs-popover", popover.clone());
            }

            // Store widget refs by key so bind/unbind don't rely on child ordering.
            unsafe {
                list_item.set_data("fs-picture", picture.clone());
                list_item.set_data("fs-index-label", index_label.clone());
                list_item.set_data("fs-badge", badge.clone());
                list_item.set_data("fs-filename-label", filename_label.clone());
                list_item.set_data("fs-item-box", item_box.clone());
            }

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
                *widget.imp().selection_anchor.borrow_mut() = Some(path.clone());
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
                // Shift-click selects everything from the anchor to here.
                if modifiers.contains(gdk4::ModifierType::SHIFT_MASK) {
                    widget.select_range_to_path(&path);
                    return;
                }
                *widget.imp().selection_anchor.borrow_mut() = Some(path.clone());
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

            // Right-click: capture context on the pane then swap the menu model and popup.
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
                let (Some(widget), Some(popover)) =
                    (widget_weak_gesture.upgrade(), popover_weak.upgrade())
                else {
                    return;
                };
                if let Some(clicked_path) = list_item_path(&list_item) {
                    widget.open_context_menu(&popover, clicked_path);
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
            if let Some(ib) = item_box.as_ref() {
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

            // Binding never queries the database: until the worker's colour
            // map arrives, rows show no colour, and its arrival recolours them.
            let badge_color = widget_weak_bind.upgrade().and_then(|w| {
                w.imp()
                    .path_colors
                    .borrow()
                    .as_ref()
                    .and_then(|colors| colors.get(&entry.path()).cloned())
            });
            apply_pill_color(&index_label, badge_color.as_deref());

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
        imp.scroll.set_overlay_scrolling(false);
        let fast_scroll = imp.scroll.vscrollbar();
        fast_scroll.add_css_class("sharpr-fast-scroll");
        fast_scroll.set_tooltip_text(Some("Drag to jump through the images"));
        fast_scroll.update_property(&[gtk4::accessible::Property::Label("Browse images")]);
        imp.scroll.set_child(Some(&imp.list_view));

        imp.content_stack.add_named(&imp.scroll, Some("list"));
        imp.content_stack
            .add_named(&imp.empty_status, Some("empty"));

        imp.root_box.append(&imp.content_stack);
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

        self.setup_context_menu_actions();
    }

    fn setup_context_menu_actions(&self) {
        let group = self.imp().context_actions.clone();

        let open = gio::SimpleAction::new("open", None);
        {
            let w = self.downgrade();
            open.connect_activate(move |_, _| {
                let Some(widget) = w.upgrade() else { return };
                let path = widget
                    .imp()
                    .menu_ctx
                    .borrow()
                    .as_ref()
                    .map(|c| c.clicked_path.clone());
                if let Some(p) = path {
                    launch_default_for_path(&p);
                }
            });
        }
        group.add_action(&open);

        let reveal = gio::SimpleAction::new("reveal", None);
        {
            let w = self.downgrade();
            reveal.connect_activate(move |_, _| {
                let Some(widget) = w.upgrade() else { return };
                let path = widget
                    .imp()
                    .menu_ctx
                    .borrow()
                    .as_ref()
                    .map(|c| c.clicked_path.clone());
                if let Some(p) = path {
                    reveal_in_file_manager(&p);
                }
            });
        }
        group.add_action(&reveal);

        let copy_clipboard = gio::SimpleAction::new("copy-to-clipboard", None);
        {
            let w = self.downgrade();
            copy_clipboard.connect_activate(move |_, _| {
                let Some(widget) = w.upgrade() else { return };
                let path = widget
                    .imp()
                    .menu_ctx
                    .borrow()
                    .as_ref()
                    .map(|c| c.clicked_path.clone());
                if let Some(p) = path {
                    widget.copy_image_to_clipboard(&p);
                }
            });
        }
        group.add_action(&copy_clipboard);

        let add_collection = gio::SimpleAction::new("add-to-collection", None);
        {
            let w = self.downgrade();
            add_collection.connect_activate(move |_, _| {
                let Some(widget) = w.upgrade() else { return };
                let paths = widget
                    .imp()
                    .menu_ctx
                    .borrow()
                    .as_ref()
                    .map(|c| c.action_paths.clone());
                if let Some(paths) = paths {
                    widget.emit_add_to_collection_requested(paths);
                }
            });
        }
        group.add_action(&add_collection);

        let info = gio::SimpleAction::new("info", None);
        {
            let w = self.downgrade();
            info.connect_activate(move |_, _| {
                let Some(widget) = w.upgrade() else { return };
                let path = widget
                    .imp()
                    .menu_ctx
                    .borrow()
                    .as_ref()
                    .map(|c| c.clicked_path.clone());
                if let Some(path) = path {
                    if let Some(cb) = widget.imp().info_requested_cb.borrow().as_ref() {
                        cb(path);
                    }
                }
            });
        }
        group.add_action(&info);

        for (name, kind) in [
            ("crop-copy", CopyKind::Crop),
            ("export-webp", CopyKind::Export),
        ] {
            let action = gio::SimpleAction::new(name, None);
            let w = self.downgrade();
            action.connect_activate(move |_, _| {
                let Some(widget) = w.upgrade() else { return };
                let paths = widget.imp().menu_ctx.borrow().as_ref().map(|c| match kind {
                    CopyKind::Crop => vec![c.clicked_path.clone()],
                    CopyKind::Export => c.action_paths.clone(),
                });
                if let Some(paths) = paths {
                    if let Some(cb) = widget.imp().copy_requested_cb.borrow().as_ref() {
                        cb(kind, paths);
                    }
                }
            });
            group.add_action(&action);
        }

        let remove_collection = gio::SimpleAction::new("remove-from-collection", None);
        {
            let w = self.downgrade();
            remove_collection.connect_activate(move |_, _| {
                let Some(widget) = w.upgrade() else { return };
                let paths = widget
                    .imp()
                    .menu_ctx
                    .borrow()
                    .as_ref()
                    .map(|c| c.action_paths.clone());
                if let Some(paths) = paths {
                    widget.emit_remove_from_collection_requested(paths);
                }
            });
        }
        group.add_action(&remove_collection);

        let copy_folder = gio::SimpleAction::new("copy-to-folder", None);
        {
            let w = self.downgrade();
            copy_folder.connect_activate(move |_, _| {
                let Some(widget) = w.upgrade() else { return };
                let paths = widget
                    .imp()
                    .menu_ctx
                    .borrow()
                    .as_ref()
                    .map(|c| c.action_paths.clone());
                if let Some(paths) = paths {
                    widget.emit_copy_to_folder_requested(paths);
                }
            });
        }
        group.add_action(&copy_folder);

        let move_folder = gio::SimpleAction::new("move-to-folder", None);
        {
            let w = self.downgrade();
            move_folder.connect_activate(move |_, _| {
                let Some(widget) = w.upgrade() else { return };
                let paths = widget
                    .imp()
                    .menu_ctx
                    .borrow()
                    .as_ref()
                    .map(|c| c.action_paths.clone());
                if let Some(paths) = paths {
                    widget.emit_move_to_folder_requested(paths);
                }
            });
        }
        group.add_action(&move_folder);

        let rename = gio::SimpleAction::new("rename", None);
        {
            let w = self.downgrade();
            rename.connect_activate(move |_, _| {
                let Some(widget) = w.upgrade() else { return };
                let path = widget
                    .imp()
                    .menu_ctx
                    .borrow()
                    .as_ref()
                    .map(|c| c.clicked_path.clone());
                if let Some(p) = path {
                    widget.emit_rename_requested(p);
                }
            });
        }
        group.add_action(&rename);

        let duplicate = gio::SimpleAction::new("duplicate", None);
        {
            let w = self.downgrade();
            duplicate.connect_activate(move |_, _| {
                let Some(widget) = w.upgrade() else { return };
                let path = widget
                    .imp()
                    .menu_ctx
                    .borrow()
                    .as_ref()
                    .map(|c| c.clicked_path.clone());
                if let Some(p) = path {
                    widget.emit_duplicate_requested(p);
                }
            });
        }
        group.add_action(&duplicate);

        let repeat = gio::SimpleAction::new("add-last-bucket", None);
        let weak = self.downgrade();
        repeat.connect_activate(move |_, _| {
            let Some(strip) = weak.upgrade() else { return };
            let paths = strip
                .imp()
                .menu_ctx
                .borrow()
                .as_ref()
                .map(|c| c.action_paths.clone())
                .unwrap_or_default();
            if let Some(window) = strip
                .root()
                .and_downcast::<crate::ui::window::SharprWindow>()
            {
                window.add_to_last_bucket(paths);
            }
        });
        group.add_action(&repeat);
        let trash = gio::SimpleAction::new("trash", None);
        {
            let w = self.downgrade();
            trash.connect_activate(move |_, _| {
                let Some(widget) = w.upgrade() else { return };
                let paths = widget
                    .imp()
                    .menu_ctx
                    .borrow()
                    .as_ref()
                    .map(|c| c.action_paths.clone());
                if let Some(paths) = paths {
                    widget.emit_trash_requested(paths);
                }
            });
        }
        group.add_action(&trash);

        self.insert_action_group("filmstrip", Some(&group));
    }

    /// Title of the empty state, so an empty search result does not read
    /// like an empty folder.
    pub fn set_empty_title(&self, title: &str) {
        self.imp().empty_status.set_title(title);
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

        if store.n_items() == 0 {
            imp.content_stack.set_visible_child_name("empty");
        } else {
            imp.content_stack.set_visible_child_name("list");
        }

        self.schedule_visible_thumbnails();
        if imp.refresh_schedule_pending.replace(true) {
            return;
        }
        let widget = self.downgrade();
        glib::timeout_add_local(
            std::time::Duration::from_millis(THUMBNAIL_RESCHEDULE_DEBOUNCE_MS),
            move || {
                let Some(filmstrip) = widget.upgrade() else {
                    return glib::ControlFlow::Break;
                };
                let page_size = filmstrip.imp().scroll.vadjustment().page_size();
                filmstrip.schedule_visible_thumbnails();
                if page_size > 0.0 {
                    filmstrip.imp().refresh_schedule_pending.set(false);
                    glib::ControlFlow::Break
                } else {
                    glib::ControlFlow::Continue
                }
            },
        );
    }

    pub fn scroll_position(&self) -> f64 {
        self.imp().scroll.vadjustment().value()
    }

    pub fn restore_scroll_position(&self, position: f64) {
        let weak = self.downgrade();
        gtk4::glib::idle_add_local_once(move || {
            if let Some(filmstrip) = weak.upgrade() {
                filmstrip.imp().scroll.vadjustment().set_value(position);
            }
        });
    }

    pub fn refresh_virtual(&self) {
        self.refresh();
    }

    pub fn clear_model(&self) {
        self.imp()
            .selection_model
            .set_model(None::<&gio::ListStore>);
    }

    pub fn refresh_collection_colors(&self, collections: &[crate::library_index::Collection]) {
        *self.imp().bucket_collections.borrow_mut() = collections.to_vec();
        self.rebuild_path_colors();
        self.imp().list_view.queue_draw();
    }

    pub fn set_library_index(&self, index: Arc<crate::library_index::LibraryIndex>) {
        *self.imp().cached_index.borrow_mut() = Some(index);
        self.rebuild_path_colors();
    }

    /// Rebuilds the path → badge colour map on a worker from bucket
    /// membership (`buckets::path_colors`). Stale builds are dropped by
    /// generation.
    pub fn rebuild_path_colors(&self) {
        let Some(index) = self.imp().cached_index.borrow().clone() else {
            return;
        };
        let collections = self.imp().bucket_collections.borrow().clone();
        let generation = self.imp().path_colors_generation.get().wrapping_add(1);
        self.imp().path_colors_generation.set(generation);
        let (tx, rx) = async_channel::bounded(1);
        std::thread::spawn(
            move || match crate::buckets::path_colors(&index, &collections) {
                Ok(colors) => {
                    let _ = tx.send_blocking(colors);
                }
                Err(error) => crate::bench_warn!(
                    "filmstrip.path_colors.fail",
                    serde_json::json!({"error": error})
                ),
            },
        );
        let weak = self.downgrade();
        glib::MainContext::default().spawn_local(async move {
            let Ok(colors) = rx.recv().await else { return };
            let Some(filmstrip) = weak.upgrade() else {
                return;
            };
            if filmstrip.imp().path_colors_generation.get() == generation {
                sync_visible_pill_colors(filmstrip.imp().list_view.upcast_ref(), &colors);
                *filmstrip.imp().path_colors.borrow_mut() = Some(colors);
            }
        });
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
        // task if it's the first pending notification. A short debounce keeps
        // large thumbnail batches from rescheduling the filmstrip on every
        // single completion.
        if imp.pending_notify_count.fetch_add(1, Ordering::Relaxed) == 0 {
            let widget_weak = self.downgrade();
            glib::timeout_add_local(
                std::time::Duration::from_millis(THUMBNAIL_RESCHEDULE_DEBOUNCE_MS),
                move || {
                    if let Some(widget) = widget_weak.upgrade() {
                        widget
                            .imp()
                            .pending_notify_count
                            .store(0, Ordering::Relaxed);
                        widget.schedule_visible_thumbnails();
                    }
                    glib::ControlFlow::Break
                },
            );
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
        let scroll_value = adjustment.value();
        let page_size = adjustment.page_size();
        let visible_start = (scroll_value / ESTIMATED_ROW_HEIGHT).floor().max(0.0) as u32;
        let visible_rows = if page_size > 0.0 {
            (page_size / ESTIMATED_ROW_HEIGHT).ceil() as u32
        } else {
            0
        };
        let layout_ready = page_size > 0.0;

        // Throttling: only scan the buffer if the scroll has moved significantly
        // or enough time has passed. We ALWAYS scan the visible range.
        let scroll_delta = (scroll_value - imp.last_scroll_value.get()).abs();
        let time_since_buffer_scan = imp
            .last_buffer_scan_time
            .get()
            .map_or(std::time::Duration::MAX, |t| t.elapsed());

        let should_scan_buffer = layout_ready
            && (scroll_delta > ESTIMATED_ROW_HEIGHT
                || time_since_buffer_scan > std::time::Duration::from_millis(500));

        if should_scan_buffer {
            imp.last_scroll_value.set(scroll_value);
            imp.last_buffer_scan_time
                .set(Some(std::time::Instant::now()));
        }

        let mut visible_range_start = visible_start;
        let mut visible_range_end = visible_start.saturating_add(visible_rows);
        let preload_range_start = visible_start.saturating_sub(BUFFER_ROWS);
        let preload_range_end = visible_start
            .saturating_add(visible_rows)
            .saturating_add(BUFFER_ROWS);

        let image_count = state_rc.borrow().library.image_count();
        let mut visible_capped_end = visible_range_end.min(image_count);

        if visible_rows == 0 || visible_capped_end <= visible_range_start {
            visible_range_start = 0;
            visible_range_end = FALLBACK_VISIBLE_ROWS;
            visible_capped_end = visible_range_end.min(image_count);
        }

        let mut visible_worker_paths: Vec<PathBuf> = Vec::new();
        let mut preload_worker_paths: Vec<PathBuf> = Vec::new();

        {
            let state = state_rc.borrow();

            // 1. Always collect visible paths.
            for index in visible_range_start..visible_capped_end {
                let Some(entry) = state.library.entry_at(index) else {
                    continue;
                };
                if entry.thumbnail().is_none() && !entry.thumbnail_failed() {
                    visible_worker_paths.push(entry.path());
                }
            }

            // 2. Collect buffer paths if throttled scan is allowed.
            if should_scan_buffer {
                let preload_capped_end = preload_range_end.min(image_count);
                for index in preload_range_start..preload_capped_end {
                    // Skip the visible range we already checked.
                    if index >= visible_range_start && index < visible_capped_end {
                        continue;
                    }
                    let Some(entry) = state.library.entry_at(index) else {
                        continue;
                    };
                    if entry.thumbnail().is_none() && !entry.thumbnail_failed() {
                        preload_worker_paths.push(entry.path());
                    }
                }

                // 3. Trickle-scan: If we have space in the enqueue pass, look further out.
                // This ensures large folders eventually load fully.
                if preload_worker_paths.len() < MAX_PRELOAD_ENQUEUE_PER_PASS {
                    // Look forward first
                    let mut idx = preload_capped_end;
                    while idx < image_count
                        && preload_worker_paths.len() < MAX_PRELOAD_ENQUEUE_PER_PASS
                    {
                        if let Some(entry) = state.library.entry_at(idx) {
                            if entry.thumbnail().is_none() && !entry.thumbnail_failed() {
                                preload_worker_paths.push(entry.path());
                            }
                        }
                        idx += 1;
                    }

                    // Then look backward
                    let mut idx = preload_range_start;
                    while idx > 0 && preload_worker_paths.len() < MAX_PRELOAD_ENQUEUE_PER_PASS {
                        idx -= 1;
                        if let Some(entry) = state.library.entry_at(idx) {
                            if entry.thumbnail().is_none() && !entry.thumbnail_failed() {
                                preload_worker_paths.push(entry.path());
                            }
                        }
                    }
                }
            }
        }

        let visible_candidates = visible_worker_paths.len();
        let preload_candidates = preload_worker_paths.len();
        let mut visible_enqueued = 0usize;
        let mut preload_enqueued = 0usize;
        let mut visible_full = false;

        if visible_candidates > 0 {
            if let Ok(mut pending) = pending_set.lock() {
                for path in visible_worker_paths {
                    if pending.insert(path.clone()) {
                        if visible_tx
                            .try_send(WorkerRequest::Thumbnail {
                                path: path.clone(),
                                gen,
                            })
                            .is_err()
                        {
                            pending.remove(&path);
                            visible_full = true;
                            // Don't break here, try others, but if it's full it's likely full for all.
                        } else {
                            visible_enqueued += 1;
                        }
                    }
                }
            }
        }

        if preload_candidates > 0 {
            if let Ok(mut pending) = pending_set.lock() {
                for path in preload_worker_paths
                    .into_iter()
                    .take(MAX_PRELOAD_ENQUEUE_PER_PASS)
                {
                    if pending.insert(path.clone()) {
                        if preload_tx
                            .try_send(WorkerRequest::Thumbnail {
                                path: path.clone(),
                                gen,
                            })
                            .is_err()
                        {
                            pending.remove(&path);
                            break; // Preload can break, it's low priority.
                        } else {
                            preload_enqueued += 1;
                        }
                    }
                }
            }

            // If we hit the batch limit, there's likely more work to do.
            // Schedule another pass soon to keep the workers busy and the pill visible.
            if preload_enqueued >= MAX_PRELOAD_ENQUEUE_PER_PASS {
                let widget_weak = self.downgrade();
                glib::timeout_add_local(
                    std::time::Duration::from_millis(PRELOAD_CHAIN_DELAY_MS),
                    move || {
                        if let Some(filmstrip) = widget_weak.upgrade() {
                            filmstrip.schedule_visible_thumbnails();
                        }
                        glib::ControlFlow::Break
                    },
                );
            }
        }

        // If we failed to enqueue any visible thumbnails because the channel was full,
        // we MUST schedule a retry, otherwise we might deadlock if no more events happen.
        if visible_candidates > visible_enqueued && visible_full {
            let widget_weak = self.downgrade();
            glib::timeout_add_local(std::time::Duration::from_millis(100), move || {
                if let Some(filmstrip) = widget_weak.upgrade() {
                    filmstrip.schedule_visible_thumbnails();
                }
                glib::ControlFlow::Break
            });
        }

        if visible_candidates > 0 || preload_candidates > 0 {
            crate::bench_event!(
                "filmstrip.thumbnail_schedule",
                serde_json::json!({
                    "image_count": image_count,
                    "visible_start": visible_range_start,
                    "visible_end": visible_capped_end,
                    "preload_start": preload_range_start,
                    "preload_end": preload_range_end.min(image_count),
                    "visible_candidates": visible_candidates,
                    "preload_candidates": preload_candidates,
                    "visible_enqueued": visible_enqueued,
                    "preload_enqueued": preload_enqueued,
                    "visible_full": visible_full,
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

    pub fn connect_trash_requested<F: Fn(Vec<PathBuf>) + 'static>(&self, f: F) {
        *self.imp().trash_requested_cb.borrow_mut() = Some(Box::new(f));
    }

    pub fn set_sort_order_changed_cb<F: Fn(SortOrder) + 'static>(&self, cb: F) {
        *self.imp().sort_order_changed_cb.borrow_mut() = Some(Box::new(cb));
    }

    pub fn connect_save_search_as_collection<F: Fn(&str) + 'static>(&self, f: F) {
        *self.imp().save_search_as_collection_cb.borrow_mut() = Some(Box::new(f));
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

    fn emit_trash_requested(&self, paths: Vec<PathBuf>) {
        if let Some(cb) = self.imp().trash_requested_cb.borrow().as_ref() {
            cb(paths);
        }
    }

    /// Shows the image menu for `clicked_path` from its row's popover.
    fn open_context_menu(&self, popover: &gtk4::PopoverMenu, clicked_path: PathBuf) {
        let (in_collection, has_library_index) = self
            .imp()
            .state
            .borrow()
            .as_ref()
            .map(|s| {
                let s = s.borrow();
                (
                    matches!(s.scope, ViewScope::Collection(_)),
                    s.library_index.is_some(),
                )
            })
            .unwrap_or((false, false));
        let action_paths = collection_action_paths(self, &clicked_path);
        let is_multi = action_paths.len() > 1;
        *self.imp().menu_ctx.borrow_mut() = Some(MenuContext {
            clicked_path,
            action_paths,
        });
        self.action_set_enabled("filmstrip.add-to-collection", has_library_index);
        let last_name = self.imp().state.borrow().as_ref().and_then(|state| {
            let state = state.borrow();
            state.settings.last_bucket().and_then(|id| {
                state
                    .collections
                    .iter()
                    .find(|c| c.id == id)
                    .map(|c| c.name.clone())
            })
        });
        popover.set_menu_model(Some(&build_context_menu_model(
            in_collection,
            is_multi,
            last_name.as_deref(),
        )));
        popover.popup();
    }

    /// Viewer and filmstrip share this menu model, context and action group.
    pub(crate) fn attach_viewer_menu(&self, viewer: &super::viewer::ViewerPane) {
        viewer.insert_action_group("filmstrip", Some(&self.imp().context_actions));
        viewer.imp().image_menu.set_has_arrow(false);
        viewer.imp().image_menu.set_parent(viewer);
        let gesture = gtk4::GestureClick::new();
        gesture.set_button(3);
        let weak_strip = self.downgrade();
        let weak_viewer = viewer.downgrade();
        gesture.connect_pressed(move |_, _, x, y| {
            if let (Some(strip), Some(viewer)) = (weak_strip.upgrade(), weak_viewer.upgrade()) {
                strip.open_viewer_menu(&viewer, x, y);
            }
        });
        viewer.add_controller(gesture);
    }
    pub(crate) fn open_viewer_menu(
        &self,
        viewer: &super::viewer::ViewerPane,
        x: f64,
        y: f64,
    ) -> bool {
        if viewer
            .root()
            .is_some_and(|root| super::window::file_actions_blocked(root.upcast_ref()))
        {
            return false;
        }
        let Some(path) = viewer.current_path() else {
            return false;
        };
        let popover = &viewer.imp().image_menu;
        popover.set_pointing_to(Some(&gdk4::Rectangle::new(x as i32, y as i32, 1, 1)));
        self.open_context_menu(popover, path);
        true
    }

    /// Opens the image menu for the current image from the keyboard
    /// (Shift+F10 or the Menu key). Returns false when no row is on screen.
    pub fn open_context_menu_for_current(&self) -> bool {
        if !self.is_mapped() {
            return false;
        }
        let Some(path) = self
            .imp()
            .state
            .borrow()
            .as_ref()
            .and_then(|s| s.borrow().library.selected_entry())
            .map(|entry| entry.path())
        else {
            return false;
        };
        let mut row = self.imp().list_view.first_child();
        while let Some(current) = row {
            if let Some(item_box) = current.first_child() {
                let matches = unsafe {
                    item_box
                        .data::<PathBuf>("fs-path")
                        .is_some_and(|p| p.as_ref() == &path)
                };
                let popover = unsafe {
                    item_box
                        .data::<gtk4::PopoverMenu>("fs-popover")
                        .map(|p| p.as_ref().clone())
                };
                if let (true, Some(popover)) = (matches, popover) {
                    let Some(bounds) = item_box.compute_bounds(&self.imp().scroll) else {
                        return false;
                    };
                    if bounds.y() + bounds.height() <= 0.0
                        || bounds.y() >= self.imp().scroll.height() as f32
                    {
                        return false;
                    }
                    self.open_context_menu(&popover, path);
                    return true;
                }
            }
            row = current.next_sibling();
        }
        false
    }

    pub fn connect_info_requested<F: Fn(PathBuf) + 'static>(&self, f: F) {
        *self.imp().info_requested_cb.borrow_mut() = Some(Box::new(f));
    }

    pub fn connect_copy_requested<F: Fn(CopyKind, Vec<PathBuf>) + 'static>(&self, f: F) {
        *self.imp().copy_requested_cb.borrow_mut() = Some(Box::new(f));
    }

    /// Hands off the sort header/menu/actions so the window can install its
    /// combined Sort and Filter control, whose filters add search chips.
    pub fn take_header_buttons(
        &self,
    ) -> Option<(
        libadwaita::HeaderBar,
        gio::MenuModel,
        gio::SimpleActionGroup,
    )> {
        let imp = self.imp();
        let header = imp.header_bar.borrow().clone()?;
        let sort = imp.sort_btn.borrow().clone()?;
        let actions = imp.action_group.borrow().clone()?;
        let menu = sort.menu_model()?;
        header.remove(&sort);
        Some((header, menu, actions))
    }

    pub fn connect_add_to_collection_requested<F: Fn(Vec<PathBuf>) + 'static>(&self, f: F) {
        *self.imp().add_to_collection_requested_cb.borrow_mut() = Some(Box::new(f));
    }

    pub fn connect_action_selection_changed<F: Fn(usize) + 'static>(&self, f: F) {
        *self.imp().action_selection_changed_cb.borrow_mut() = Some(Box::new(f));
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

    pub fn connect_copy_to_folder_requested<F: Fn(Vec<PathBuf>) + 'static>(&self, f: F) {
        *self.imp().copy_to_folder_requested_cb.borrow_mut() = Some(Box::new(f));
    }

    pub fn connect_move_to_folder_requested<F: Fn(Vec<PathBuf>) + 'static>(&self, f: F) {
        *self.imp().move_to_folder_requested_cb.borrow_mut() = Some(Box::new(f));
    }

    pub fn connect_rename_requested<F: Fn(PathBuf) + 'static>(&self, f: F) {
        *self.imp().rename_requested_cb.borrow_mut() = Some(Box::new(f));
    }

    pub fn connect_duplicate_requested<F: Fn(PathBuf) + 'static>(&self, f: F) {
        *self.imp().duplicate_requested_cb.borrow_mut() = Some(Box::new(f));
    }

    fn emit_copy_to_folder_requested(&self, paths: Vec<PathBuf>) {
        if let Some(cb) = self.imp().copy_to_folder_requested_cb.borrow().as_ref() {
            cb(paths);
        }
    }

    fn emit_move_to_folder_requested(&self, paths: Vec<PathBuf>) {
        if let Some(cb) = self.imp().move_to_folder_requested_cb.borrow().as_ref() {
            cb(paths);
        }
    }

    pub fn emit_rename_requested(&self, path: PathBuf) {
        if let Some(cb) = self.imp().rename_requested_cb.borrow().as_ref() {
            cb(path);
        }
    }

    pub fn emit_duplicate_requested(&self, path: PathBuf) {
        if let Some(cb) = self.imp().duplicate_requested_cb.borrow().as_ref() {
            cb(path);
        }
    }

    /// Returns the paths that should be acted on from a keyboard shortcut: the
    /// full multi-selection if more than one item is selected, otherwise the
    /// single currently focused image.
    /// Opens the bucket picker for the current selection (Ctrl+B).
    /// Opens WebP export for the current selection (Ctrl+E).
    pub fn request_export_selection(&self) -> bool {
        let paths = self.selected_paths_for_action();
        if paths.is_empty() {
            return false;
        }
        if let Some(cb) = self.imp().copy_requested_cb.borrow().as_ref() {
            cb(CopyKind::Export, paths);
        }
        true
    }

    pub fn request_add_selection_to_bucket(&self) -> bool {
        let paths = self.selected_paths_for_action();
        if paths.is_empty() {
            return false;
        }
        self.emit_add_to_collection_requested(paths);
        true
    }

    /// Opens the bucket picker for `paths`, such as chosen search results.
    pub fn request_add_paths_to_bucket(&self, paths: Vec<PathBuf>) {
        if !paths.is_empty() {
            self.emit_add_to_collection_requested(paths);
        }
    }

    /// Selects every image from the selection anchor (the last plain or
    /// Ctrl-click, or else the current image) to `path`, inclusive.
    pub fn select_range_to_path(&self, path: &Path) {
        let anchor = self.imp().selection_anchor.borrow().clone();
        if let Some(state) = self.imp().state.borrow().as_ref() {
            let mut state = state.borrow_mut();
            let anchor = anchor
                .or_else(|| state.library.selected_entry().map(|e| e.path()))
                .and_then(|anchor| state.library.index_of_path(&anchor));
            let Some(end) = state.library.index_of_path(path) else {
                return;
            };
            let start = anchor.unwrap_or(end);
            let range: Vec<PathBuf> = (start.min(end)..=start.max(end))
                .filter_map(|i| state.library.entry_at(i).map(|e| e.path()))
                .collect();
            state.selected_paths = range.into_iter().collect();
            state.selection_explicit = true;
        }
        self.refresh_multi_selection_visuals();
    }

    /// Selects every image shown (Ctrl+A).
    pub fn select_all(&self) {
        if let Some(state) = self.imp().state.borrow().as_ref() {
            let mut state = state.borrow_mut();
            let all: Vec<PathBuf> = (0..state.library.image_count())
                .filter_map(|i| state.library.entry_at(i).map(|e| e.path()))
                .collect();
            state.selected_paths = all.into_iter().collect();
            state.selection_explicit = true;
        }
        self.refresh_multi_selection_visuals();
    }

    pub fn selected_paths_for_action(&self) -> Vec<PathBuf> {
        self.imp()
            .state
            .borrow()
            .as_ref()
            .and_then(|s| {
                let s = s.borrow();
                if s.selected_paths.len() > 1
                    || (s.selection_explicit && !s.selected_paths.is_empty())
                {
                    Some(s.selected_paths.iter().cloned().collect())
                } else {
                    s.library.selected_entry().map(|e| vec![e.path()])
                }
            })
            .unwrap_or_default()
    }

    pub fn selected_single_path(&self) -> Option<PathBuf> {
        self.imp()
            .state
            .borrow()
            .as_ref()
            .and_then(|s| s.borrow().library.selected_entry().map(|e| e.path()))
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
            state.selection_explicit = false;
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
            state.selection_explicit = true;
        }
        self.refresh_multi_selection_visuals();
    }

    pub fn refresh_multi_selection_visuals(&self) {
        let (selected_paths, explicit) = self
            .imp()
            .state
            .borrow()
            .as_ref()
            .and_then(|state| state.try_borrow().ok())
            .map(|state| (state.selected_paths.clone(), state.selection_explicit))
            .unwrap_or_default();
        // A single image chosen on purpose is marked too: actions use it.
        let show_multi = selected_paths.len() > 1 || (explicit && !selected_paths.is_empty());
        if let Some(callback) = self.imp().action_selection_changed_cb.borrow().as_ref() {
            callback(selected_paths.len());
        }
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

/// Sets the bucket colour class of a row's index pill, replacing the class a
/// recycled row had before.
fn apply_pill_color(label: &gtk4::Label, color: Option<&str>) {
    let old: Option<String> = unsafe { label.steal_data("fs-pill-class") };
    if let Some(class) = old {
        label.remove_css_class(&class);
    }
    if let Some(color) = color {
        let class = register_pill_color(color);
        label.add_css_class(&class);
        unsafe { label.set_data("fs-pill-class", class) };
    }
}

/// Recolours the pills of rows already bound, e.g. after a bucket edit
/// changes a visible image's colour.
fn sync_visible_pill_colors(widget: &gtk4::Widget, colors: &HashMap<PathBuf, String>) {
    fn find_pill(widget: &gtk4::Widget) -> Option<gtk4::Label> {
        if widget.has_css_class("filmstrip-index-label") {
            return widget.downcast_ref::<gtk4::Label>().cloned();
        }
        let mut child = widget.first_child();
        while let Some(current) = child {
            if let Some(label) = find_pill(&current) {
                return Some(label);
            }
            child = current.next_sibling();
        }
        None
    }
    let path = unsafe {
        widget
            .data::<PathBuf>("fs-path")
            .map(|p| p.as_ref().clone())
    };
    if let Some(path) = path {
        if let Some(label) = find_pill(widget) {
            apply_pill_color(&label, colors.get(&path).map(String::as_str));
        }
        return;
    }
    let mut child = widget.first_child();
    while let Some(current) = child {
        sync_visible_pill_colors(&current, colors);
        child = current.next_sibling();
    }
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

impl FilmstripPane {
    fn copy_image_to_clipboard(&self, path: &Path) {
        let visible = |path: &Path| {
            self.imp().state.borrow().as_ref().is_some_and(|state| {
                let state = state.borrow();
                crate::buckets::visible(
                    path,
                    state.settings.active_library().map(|l| l.root.as_path()),
                    &state.disabled_folders,
                )
            })
        };
        if !visible(path) {
            return;
        }
        if self.imp().clipboard_decoder.borrow().is_none() {
            let (decoder, results) = crate::image_pipeline::clipboard::Decoder::start();
            *self.imp().clipboard_decoder.borrow_mut() = Some(decoder);
            let weak = self.downgrade();
            glib::MainContext::default().spawn_local(async move {
                while let Ok(result) = results.recv().await {
                    let Some(strip) = weak.upgrade() else { return };
                    if !strip
                        .imp()
                        .clipboard_decoder
                        .borrow()
                        .as_ref()
                        .is_some_and(|d| d.is_current(result.generation))
                    {
                        continue;
                    }
                    let visible = strip.imp().state.borrow().as_ref().is_some_and(|state| {
                        let state = state.borrow();
                        crate::buckets::visible(
                            &result.path,
                            state.settings.active_library().map(|l| l.root.as_path()),
                            &state.disabled_folders,
                        )
                    });
                    if !visible {
                        continue;
                    }
                    match result.image {
                        Ok(image) => {
                            let texture = gdk4::MemoryTexture::new(
                                image.width as i32,
                                image.height as i32,
                                gdk4::MemoryFormat::R8g8b8a8,
                                &glib::Bytes::from_owned(image.rgba),
                                image.width as usize * 4,
                            );
                            strip.clipboard().set_texture(&texture);
                        }
                        Err(error) => {
                            if let Some(window) =
                                strip.root().and_downcast::<super::window::SharprWindow>()
                            {
                                window.add_toast(libadwaita::Toast::new(&format!(
                                    "Could not copy image: {error}"
                                )));
                            }
                        }
                    }
                }
            });
        }
        if let Some(decoder) = self.imp().clipboard_decoder.borrow().as_ref() {
            decoder.request(path.to_path_buf());
        }
    }
}

fn reveal_in_file_manager(path: &Path) {
    let uri = gio::File::for_path(path).uri().to_string();
    let parent_uri = path
        .parent()
        .map(|p| gio::File::for_path(p).uri().to_string());
    glib::MainContext::default().spawn_local(async move {
        let shown = async {
            let conn = gio::bus_get_future(gio::BusType::Session).await.ok()?;
            let uris = glib::Variant::array_from_iter::<String>(std::iter::once(uri.to_variant()));
            let params = glib::Variant::tuple_from_iter([uris, "".to_variant()]);
            conn.call_future(
                Some("org.freedesktop.FileManager1"),
                "/org/freedesktop/FileManager1",
                "org.freedesktop.FileManager1",
                "ShowItems",
                Some(&params),
                None,
                gio::DBusCallFlags::NONE,
                5000,
            )
            .await
            .ok()
        }
        .await;
        if shown.is_none() {
            if let Some(u) = parent_uri {
                let _ =
                    gio::AppInfo::launch_default_for_uri_future(&u, gio::AppLaunchContext::NONE)
                        .await;
            }
        }
    });
}
