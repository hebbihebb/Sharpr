use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::Once;

use gtk4::gdk;
use gtk4::prelude::*;
use gtk4::subclass::prelude::*;

use crate::library_index::Collection;
use crate::tags::TagDatabase;
use crate::ui::tag_card::TagCard;

type TagsActivatedCallback = Box<dyn Fn(TagActivation) + 'static>;
type TagCreateCollectionCallback = Box<dyn Fn(&str) + 'static>;
type PreviewResolver = Box<dyn Fn(&Path) -> Option<gdk::Texture> + 'static>;
type PreviewRequester = Box<dyn Fn(&Path) + 'static>;

#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum BrowserMode {
    #[default]
    Grid,
    List,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct TagViewModel {
    name: String,
    count: usize,
    group_key: char,
    accent_color: Option<String>,
    preview_path: Option<PathBuf>,
    selected: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct TagGroup {
    key: char,
    tags: Vec<TagViewModel>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct SelectionModifiers {
    ctrl: bool,
    shift: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TagActivation {
    pub tags: Vec<String>,
    pub focus_viewer: bool,
}

#[derive(Clone)]
pub(crate) struct TagCardBinding {
    preview_path: Option<PathBuf>,
    card: TagCard,
}

mod imp {
    use super::*;

    pub struct TagBrowser {
        pub root_box: gtk4::Box,
        pub toolbar_box: gtk4::Box,
        pub search_entry: gtk4::SearchEntry,
        pub grid_button: gtk4::ToggleButton,
        pub list_button: gtk4::ToggleButton,
        pub clear_button: gtk4::Button,
        pub scroll: gtk4::ScrolledWindow,
        pub content_box: gtk4::Box,
        pub tags: RefCell<Option<Arc<TagDatabase>>>,
        pub collections: RefCell<Vec<Collection>>,
        pub selected_tags: RefCell<BTreeSet<String>>,
        pub selection_anchor: RefCell<Option<String>>,
        pub visible_order: RefCell<Vec<String>>,
        pub(crate) mode: Cell<BrowserMode>,
        pub preview_resolver: RefCell<Option<PreviewResolver>>,
        pub preview_requester: RefCell<Option<PreviewRequester>>,
        pub(crate) card_bindings: RefCell<Vec<TagCardBinding>>,
        pub tags_activated_cb: RefCell<Option<TagsActivatedCallback>>,
        pub tag_create_collection_cb: RefCell<Option<TagCreateCollectionCallback>>,
    }

    impl Default for TagBrowser {
        fn default() -> Self {
            let root_box = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
            let toolbar_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 12);
            let search_entry = gtk4::SearchEntry::new();
            let grid_button = gtk4::ToggleButton::builder()
                .icon_name("view-grid-symbolic")
                .tooltip_text("Grid View")
                .active(true)
                .build();
            let list_button = gtk4::ToggleButton::builder()
                .icon_name("view-list-symbolic")
                .tooltip_text("List View")
                .build();
            let clear_button = gtk4::Button::with_label("Clear all");
            let scroll = gtk4::ScrolledWindow::new();
            scroll.set_policy(gtk4::PolicyType::Never, gtk4::PolicyType::Automatic);
            scroll.set_vexpand(true);

            Self {
                root_box,
                toolbar_box,
                search_entry,
                grid_button,
                list_button,
                clear_button,
                scroll,
                content_box: gtk4::Box::new(gtk4::Orientation::Vertical, 0),
                tags: RefCell::new(None),
                collections: RefCell::new(Vec::new()),
                selected_tags: RefCell::new(BTreeSet::new()),
                selection_anchor: RefCell::new(None),
                visible_order: RefCell::new(Vec::new()),
                mode: Cell::new(BrowserMode::Grid),
                preview_resolver: RefCell::new(None),
                preview_requester: RefCell::new(None),
                card_bindings: RefCell::new(Vec::new()),
                tags_activated_cb: RefCell::new(None),
                tag_create_collection_cb: RefCell::new(None),
            }
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for TagBrowser {
        const NAME: &'static str = "SharprTagBrowser";
        type Type = super::TagBrowser;
        type ParentType = gtk4::Widget;

        fn class_init(klass: &mut Self::Class) {
            klass.set_layout_manager_type::<gtk4::BinLayout>();
        }
    }

    impl ObjectImpl for TagBrowser {
        fn dispose(&self) {
            self.scroll.unparent();
            self.root_box.unparent();
        }
    }

    impl WidgetImpl for TagBrowser {}
}

glib::wrapper! {
    pub struct TagBrowser(ObjectSubclass<imp::TagBrowser>)
        @extends gtk4::Widget;
}

impl TagBrowser {
    pub fn new(tags: Arc<TagDatabase>) -> Self {
        install_css();
        let widget: Self = glib::Object::new();
        *widget.imp().tags.borrow_mut() = Some(tags);
        widget.build_ui();
        widget.refresh();
        widget
    }

    pub fn set_collections(&self, collections: Vec<Collection>) {
        *self.imp().collections.borrow_mut() = collections;
        self.refresh();
    }

    pub fn set_preview_hooks<F, G>(&self, resolve: F, request: G)
    where
        F: Fn(&Path) -> Option<gdk::Texture> + 'static,
        G: Fn(&Path) + 'static,
    {
        *self.imp().preview_resolver.borrow_mut() = Some(Box::new(resolve));
        *self.imp().preview_requester.borrow_mut() = Some(Box::new(request));
        self.refresh();
    }

    pub fn connect_tags_activated<F: Fn(TagActivation) + 'static>(&self, f: F) {
        *self.imp().tags_activated_cb.borrow_mut() = Some(Box::new(f));
    }

    pub fn connect_tag_create_collection_requested<F: Fn(&str) + 'static>(&self, f: F) {
        *self.imp().tag_create_collection_cb.borrow_mut() = Some(Box::new(f));
    }

    pub fn refresh_preview_for_path(&self, path: &Path) {
        let resolver_binding = self.imp().preview_resolver.borrow();
        let Some(resolver) = resolver_binding.as_ref() else {
            return;
        };
        for binding in self.imp().card_bindings.borrow().iter() {
            if binding.preview_path.as_deref() == Some(path) {
                let texture = resolver(path);
                binding.card.set_preview_texture(texture.as_ref());
            }
        }
    }

    pub fn refresh(&self) {
        let imp = self.imp();
        imp.card_bindings.borrow_mut().clear();
        while let Some(child) = imp.content_box.first_child() {
            imp.content_box.remove(&child);
        }

        let Some(tags_db) = imp.tags.borrow().as_ref().cloned() else {
            self.show_empty_state("Tags are unavailable.");
            return;
        };

        let existing_tags: BTreeSet<String> = tags_db
            .all_tags()
            .into_iter()
            .filter(|(_, count)| *count >= 1)
            .map(|(tag, _)| tag)
            .collect();
        imp.selected_tags
            .borrow_mut()
            .retain(|tag| existing_tags.contains(tag));
        if imp
            .selection_anchor
            .borrow()
            .as_ref()
            .is_some_and(|anchor| !existing_tags.contains(anchor))
        {
            *imp.selection_anchor.borrow_mut() = None;
        }

        let models = build_tag_view_models(
            &tags_db,
            &collection_tag_color_map(&imp.collections.borrow()),
            &imp.selected_tags.borrow(),
        );

        if models.is_empty() {
            self.show_empty_state("No tags yet.\nSelect an image and press Ctrl+T to add tags.");
            self.sync_toolbar_state();
            return;
        }

        let groups = filter_and_group_tags(&models, imp.search_entry.text().as_str());
        let visible_order: Vec<String> = groups
            .iter()
            .flat_map(|group| group.tags.iter().map(|tag| tag.name.clone()))
            .collect();
        *imp.visible_order.borrow_mut() = visible_order;

        self.sync_toolbar_state();

        if groups.is_empty() {
            self.show_empty_state("No tags match this search.");
            return;
        }

        for group in groups {
            let heading = gtk4::Label::new(Some(&group.key.to_string()));
            heading.add_css_class("caption-heading");
            heading.add_css_class("tag-browser-group-heading");
            heading.set_halign(gtk4::Align::Start);
            imp.content_box.append(&heading);

            match imp.mode.get() {
                BrowserMode::Grid => self.append_grid_group(group),
                BrowserMode::List => self.append_list_group(group),
            }
        }
    }

    fn build_ui(&self) {
        let imp = self.imp();

        imp.root_box.add_css_class("tag-browser-root");
        imp.root_box.set_vexpand(true);

        imp.toolbar_box.add_css_class("tag-browser-toolbar");
        imp.toolbar_box.set_margin_top(16);
        imp.toolbar_box.set_margin_bottom(12);
        imp.toolbar_box.set_margin_start(18);
        imp.toolbar_box.set_margin_end(18);

        imp.search_entry.set_hexpand(true);
        imp.search_entry.set_placeholder_text(Some("Search tags"));

        let mode_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
        mode_box.add_css_class("linked");
        mode_box.append(&imp.grid_button);
        mode_box.append(&imp.list_button);

        imp.clear_button.add_css_class("flat");
        imp.clear_button.set_visible(false);

        imp.toolbar_box.append(&imp.search_entry);
        imp.toolbar_box.append(&mode_box);
        imp.toolbar_box.append(&imp.clear_button);

        imp.content_box.add_css_class("tag-browser-content");
        imp.content_box.set_margin_start(18);
        imp.content_box.set_margin_end(18);
        imp.content_box.set_margin_bottom(24);
        imp.scroll.set_child(Some(&imp.content_box));

        imp.root_box.append(&imp.toolbar_box);
        imp.root_box.append(&imp.scroll);
        imp.root_box.set_parent(self);

        let widget_weak = self.downgrade();
        imp.search_entry.connect_search_changed(move |_| {
            let Some(widget) = widget_weak.upgrade() else {
                return;
            };
            widget.refresh();
        });

        let widget_weak = self.downgrade();
        imp.grid_button.connect_toggled(move |button| {
            if !button.is_active() {
                return;
            }
            let Some(widget) = widget_weak.upgrade() else {
                return;
            };
            widget.imp().list_button.set_active(false);
            widget.imp().mode.set(BrowserMode::Grid);
            widget.refresh();
        });

        let widget_weak = self.downgrade();
        imp.list_button.connect_toggled(move |button| {
            if !button.is_active() {
                return;
            }
            let Some(widget) = widget_weak.upgrade() else {
                return;
            };
            widget.imp().grid_button.set_active(false);
            widget.imp().mode.set(BrowserMode::List);
            widget.refresh();
        });

        let widget_weak = self.downgrade();
        imp.clear_button.connect_clicked(move |_| {
            let Some(widget) = widget_weak.upgrade() else {
                return;
            };
            widget.imp().selected_tags.borrow_mut().clear();
            *widget.imp().selection_anchor.borrow_mut() = None;
            widget.imp().search_entry.set_text("");
            widget.emit_tags_activated(SelectionModifiers::default());
            widget.refresh();
        });
    }

    fn append_grid_group(&self, group: TagGroup) {
        let flow = gtk4::FlowBox::new();
        flow.set_selection_mode(gtk4::SelectionMode::None);
        flow.set_row_spacing(12);
        flow.set_column_spacing(12);
        flow.set_homogeneous(false);
        flow.set_max_children_per_line(6);
        flow.set_min_children_per_line(1);
        flow.set_margin_bottom(14);
        flow.set_halign(gtk4::Align::Start);

        for tag in group.tags {
            let child = gtk4::FlowBoxChild::new();
            child.set_halign(gtk4::Align::Start);
            child.set_valign(gtk4::Align::Start);
            let card = TagCard::new();
            card.set_label(&tag.name);
            card.set_count(tag.count);
            card.set_selected(tag.selected);
            card.set_accent_color(tag.accent_color.as_deref());

            if let Some(path) = tag.preview_path.as_deref() {
                let preview = self.resolve_preview(path);
                card.set_preview_texture(preview.as_ref());
                if preview.is_none() {
                    self.request_preview(path);
                }
            } else {
                card.set_preview_texture(None);
            }

            self.attach_tag_activation(card.widget().upcast_ref(), &tag.name);
            self.attach_drag_source(card.widget().upcast_ref(), &tag.name);
            self.attach_tag_menu(card.menu_button(), &tag.name);

            child.set_child(Some(card.widget()));
            flow.insert(&child, -1);
            self.imp().card_bindings.borrow_mut().push(TagCardBinding {
                preview_path: tag.preview_path,
                card,
            });
        }

        self.imp().content_box.append(&flow);
    }

    fn append_list_group(&self, group: TagGroup) {
        let list = gtk4::Box::new(gtk4::Orientation::Vertical, 6);
        list.set_margin_bottom(14);

        for tag in group.tags {
            let row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
            row.add_css_class("tag-browser-list-row");
            if tag.selected {
                row.add_css_class("selected");
            }

            let button = gtk4::Button::new();
            button.add_css_class("flat");
            button.add_css_class("tag-browser-chip");
            button.set_hexpand(true);
            button.set_halign(gtk4::Align::Fill);

            let button_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
            let icon = gtk4::Image::from_icon_name("bookmark-new-symbolic");
            if let Some(color) = tag.accent_color.as_deref() {
                apply_icon_color(&icon, color);
            }
            let label = gtk4::Label::new(Some(&tag.name));
            label.set_xalign(0.0);
            label.set_hexpand(true);
            let count = gtk4::Label::new(Some(&tag.count.to_string()));
            count.add_css_class("dim-label");
            button_box.append(&icon);
            button_box.append(&label);
            button_box.append(&count);
            button.set_child(Some(&button_box));

            self.attach_tag_activation(button.upcast_ref(), &tag.name);
            self.attach_drag_source(button.upcast_ref(), &tag.name);

            let menu_button = gtk4::MenuButton::new();
            menu_button.set_icon_name("view-more-symbolic");
            menu_button.add_css_class("flat");
            self.attach_tag_menu(&menu_button, &tag.name);

            row.append(&button);
            row.append(&menu_button);
            list.append(&row);
        }

        self.imp().content_box.append(&list);
    }

    fn attach_tag_activation(&self, widget: &gtk4::Widget, tag: &str) {
        let widget_weak = self.downgrade();
        let tag_name = tag.to_string();
        let gesture = gtk4::GestureClick::new();
        gesture.set_button(1);
        gesture.set_propagation_phase(gtk4::PropagationPhase::Capture);
        gesture.connect_released(move |gesture, _, _, _| {
            let Some(widget) = widget_weak.upgrade() else {
                return;
            };
            let state = gesture.current_event_state();
            widget.handle_tag_selection(
                &tag_name,
                SelectionModifiers {
                    ctrl: state.contains(gdk::ModifierType::CONTROL_MASK),
                    shift: state.contains(gdk::ModifierType::SHIFT_MASK),
                },
            );
        });
        widget.add_controller(gesture);
    }

    fn attach_drag_source(&self, widget: &gtk4::Widget, tag: &str) {
        let drag_source = gtk4::DragSource::new();
        drag_source.set_actions(gdk::DragAction::COPY);
        let tag_name = tag.to_string();
        drag_source.connect_prepare(move |_, _, _| {
            Some(gdk::ContentProvider::for_value(
                &format!("tag:{tag_name}").to_value(),
            ))
        });
        widget.add_controller(drag_source);
    }

    fn attach_tag_menu(&self, menu_button: &gtk4::MenuButton, tag: &str) {
        let popover = gtk4::Popover::new();
        let content = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
        content.set_margin_top(8);
        content.set_margin_bottom(8);
        content.set_margin_start(8);
        content.set_margin_end(8);

        let create_button = gtk4::Button::with_label("Create Collection");
        create_button.add_css_class("flat");
        create_button.set_halign(gtk4::Align::Fill);

        let delete_button = gtk4::Button::with_label("Delete Tag");
        delete_button.add_css_class("flat");
        delete_button.add_css_class("destructive-action");
        delete_button.set_halign(gtk4::Align::Fill);

        let widget_weak = self.downgrade();
        let tag_name = tag.to_string();
        let popover_weak = popover.downgrade();
        create_button.connect_clicked(move |_| {
            let Some(widget) = widget_weak.upgrade() else {
                return;
            };
            widget.emit_tag_create_collection_requested(&tag_name);
            if let Some(popover) = popover_weak.upgrade() {
                popover.popdown();
            }
        });

        let widget_weak = self.downgrade();
        let tag_name = tag.to_string();
        let popover_weak = popover.downgrade();
        delete_button.connect_clicked(move |_| {
            let Some(widget) = widget_weak.upgrade() else {
                return;
            };
            let Some(db) = widget.imp().tags.borrow().as_ref().cloned() else {
                return;
            };
            db.delete_tag_globally(&tag_name);
            widget.imp().selected_tags.borrow_mut().remove(&tag_name);
            if widget.imp().selection_anchor.borrow().as_deref() == Some(tag_name.as_str()) {
                *widget.imp().selection_anchor.borrow_mut() = None;
            }
            widget.emit_tags_activated(SelectionModifiers::default());
            widget.refresh();
            if let Some(popover) = popover_weak.upgrade() {
                popover.popdown();
            }
        });

        content.append(&create_button);
        content.append(&delete_button);
        popover.set_child(Some(&content));
        menu_button.set_popover(Some(&popover));
    }

    fn handle_tag_selection(&self, clicked_tag: &str, modifiers: SelectionModifiers) {
        let visible_order = self.imp().visible_order.borrow().clone();
        let anchor = self.imp().selection_anchor.borrow().clone();
        let current = self.imp().selected_tags.borrow().clone();
        let update = update_selection(current, &visible_order, anchor, clicked_tag, modifiers);
        *self.imp().selected_tags.borrow_mut() = update.selected_tags;
        *self.imp().selection_anchor.borrow_mut() = update.anchor;
        self.emit_tags_activated(modifiers);
        self.refresh();
    }

    fn resolve_preview(&self, path: &Path) -> Option<gdk::Texture> {
        self.imp()
            .preview_resolver
            .borrow()
            .as_ref()
            .and_then(|resolve| resolve(path))
    }

    fn request_preview(&self, path: &Path) {
        if let Some(request) = self.imp().preview_requester.borrow().as_ref() {
            request(path);
        }
    }

    fn emit_tags_activated(&self, modifiers: SelectionModifiers) {
        if let Some(cb) = self.imp().tags_activated_cb.borrow().as_ref() {
            cb(TagActivation {
                tags: self.imp().selected_tags.borrow().iter().cloned().collect(),
                focus_viewer: !modifiers.ctrl && !modifiers.shift,
            });
        }
    }

    fn emit_tag_create_collection_requested(&self, tag: &str) {
        if let Some(cb) = self.imp().tag_create_collection_cb.borrow().as_ref() {
            cb(tag);
        }
    }

    fn sync_toolbar_state(&self) {
        let has_selection = !self.imp().selected_tags.borrow().is_empty();
        self.imp().clear_button.set_visible(has_selection);
        self.imp()
            .grid_button
            .set_active(self.imp().mode.get() == BrowserMode::Grid);
        self.imp()
            .list_button
            .set_active(self.imp().mode.get() == BrowserMode::List);
    }

    fn show_empty_state(&self, text: &str) {
        let label = gtk4::Label::new(Some(text));
        label.set_justify(gtk4::Justification::Center);
        label.set_wrap(true);
        label.set_wrap_mode(gtk4::pango::WrapMode::WordChar);
        label.add_css_class("dim-label");
        label.add_css_class("tag-browser-empty-state");
        self.imp().content_box.append(&label);
    }
}

#[derive(Debug, PartialEq, Eq)]
struct SelectionUpdate {
    selected_tags: BTreeSet<String>,
    anchor: Option<String>,
}

fn build_tag_view_models(
    tags_db: &TagDatabase,
    tag_colors: &HashMap<String, String>,
    selected_tags: &BTreeSet<String>,
) -> Vec<TagViewModel> {
    tags_db
        .all_tags()
        .into_iter()
        .filter(|(_, count)| *count >= 1)
        .map(|(name, count)| TagViewModel {
            group_key: group_key_for_tag(&name),
            accent_color: tag_colors.get(&name).cloned(),
            preview_path: tags_db.paths_for_tag(&name).into_iter().next(),
            selected: selected_tags.contains(&name),
            name,
            count,
        })
        .collect()
}

fn filter_and_group_tags(tags: &[TagViewModel], query: &str) -> Vec<TagGroup> {
    let query = query.trim().to_lowercase();
    let mut grouped: BTreeMap<char, Vec<TagViewModel>> = BTreeMap::new();
    for tag in tags {
        if !query.is_empty() && !tag.name.to_lowercase().contains(&query) {
            continue;
        }
        grouped.entry(tag.group_key).or_default().push(tag.clone());
    }
    grouped
        .into_iter()
        .map(|(key, mut tags)| {
            tags.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
            TagGroup { key, tags }
        })
        .collect()
}

fn group_key_for_tag(tag: &str) -> char {
    tag.chars()
        .next()
        .map(|ch| ch.to_ascii_uppercase())
        .filter(|ch| ch.is_ascii_alphabetic())
        .unwrap_or('#')
}

fn update_selection(
    mut current: BTreeSet<String>,
    visible_order: &[String],
    anchor: Option<String>,
    clicked_tag: &str,
    modifiers: SelectionModifiers,
) -> SelectionUpdate {
    if modifiers.shift {
        let anchor_tag = anchor
            .filter(|candidate| visible_order.contains(candidate))
            .unwrap_or_else(|| clicked_tag.to_string());
        let mut range = BTreeSet::new();
        if let (Some(a), Some(b)) = (
            visible_order.iter().position(|tag| tag == &anchor_tag),
            visible_order.iter().position(|tag| tag == clicked_tag),
        ) {
            let (start, end) = if a <= b { (a, b) } else { (b, a) };
            for tag in &visible_order[start..=end] {
                range.insert(tag.clone());
            }
        } else {
            range.insert(clicked_tag.to_string());
        }
        if modifiers.ctrl {
            current.extend(range);
        } else {
            current = range;
        }
        return SelectionUpdate {
            selected_tags: current,
            anchor: Some(anchor_tag),
        };
    }

    if modifiers.ctrl {
        if !current.insert(clicked_tag.to_string()) {
            current.remove(clicked_tag);
        }
        return SelectionUpdate {
            selected_tags: current,
            anchor: Some(clicked_tag.to_string()),
        };
    }

    let mut selected = BTreeSet::new();
    selected.insert(clicked_tag.to_string());
    SelectionUpdate {
        selected_tags: selected,
        anchor: Some(clicked_tag.to_string()),
    }
}

fn collection_tag_color_map(collections: &[Collection]) -> HashMap<String, String> {
    let parent_by_id: HashMap<i64, Option<i64>> = collections
        .iter()
        .map(|collection| (collection.id, collection.parent_id))
        .collect();
    let collection_by_id: HashMap<i64, &Collection> = collections
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

fn root_collection_id(collection_id: i64, parent_by_id: &HashMap<i64, Option<i64>>) -> i64 {
    let mut root_id = collection_id;
    let mut current = parent_by_id.get(&collection_id).copied().flatten();
    while let Some(parent_id) = current {
        root_id = parent_id;
        current = parent_by_id.get(&parent_id).copied().flatten();
    }
    root_id
}

fn fallback_collection_color(collection_id: i64) -> &'static str {
    const PALETTE: &[&str] = &[
        "#57e389", "#62a0ea", "#ff7800", "#f5c211", "#dc8add", "#5bc8af", "#e01b24", "#9141ac",
    ];
    PALETTE[(collection_id as usize) % PALETTE.len()]
}

fn apply_icon_color(icon: &gtk4::Image, color: &str) {
    use std::sync::{LazyLock, Mutex};

    static REGISTERED: LazyLock<Mutex<std::collections::HashSet<String>>> =
        LazyLock::new(|| Mutex::new(std::collections::HashSet::new()));

    let key = color
        .replace([' ', '(', ')', ',', '#', '.'], "_")
        .to_lowercase();
    let class_name = format!("tag-browser-list-icon-{key}");

    if let Ok(mut registered) = REGISTERED.lock() {
        if registered.insert(key) {
            let provider = gtk4::CssProvider::new();
            provider.load_from_string(&format!(".{class_name} {{ color: {color}; }}"));
            if let Some(display) = gdk::Display::default() {
                gtk4::style_context_add_provider_for_display(
                    &display,
                    &provider,
                    gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
                );
            }
        }
    }

    icon.add_css_class(&class_name);
}

fn install_css() {
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        let provider = gtk4::CssProvider::new();
        provider.load_from_string(
            "
            .tag-browser-root {
                background: transparent;
            }
            .tag-browser-toolbar {
                border-spacing: 0;
            }
            .tag-browser-content {
                padding-top: 4px;
            }
            .tag-browser-group-heading {
                margin-bottom: 6px;
                opacity: 0.85;
            }
            .tag-browser-empty-state {
                margin-top: 64px;
                margin-bottom: 64px;
            }
            .tag-browser-list-row {
                border-radius: 14px;
                border: 1px solid alpha(@window_fg_color, 0.08);
                background: alpha(@window_fg_color, 0.03);
                padding: 4px;
            }
            .tag-browser-list-row.selected {
                border-color: alpha(@accent_color, 0.85);
                background: alpha(@accent_color, 0.10);
            }
            .tag-browser-chip {
                padding: 8px 12px;
            }
            ",
        );
        if let Some(display) = gdk::Display::default() {
            gtk4::style_context_add_provider_for_display(
                &display,
                &provider,
                gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
            );
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn group_key_uses_hash_for_non_letters() {
        assert_eq!(group_key_for_tag("alpha"), 'A');
        assert_eq!(group_key_for_tag("9lives"), '#');
        assert_eq!(group_key_for_tag("_misc"), '#');
    }

    #[test]
    fn filtering_hides_empty_groups_and_is_case_insensitive() {
        let tags = vec![
            TagViewModel {
                name: "alpha".into(),
                count: 1,
                group_key: 'A',
                accent_color: None,
                preview_path: None,
                selected: false,
            },
            TagViewModel {
                name: "Beta".into(),
                count: 1,
                group_key: 'B',
                accent_color: None,
                preview_path: None,
                selected: false,
            },
            TagViewModel {
                name: "beach".into(),
                count: 1,
                group_key: 'B',
                accent_color: None,
                preview_path: None,
                selected: false,
            },
        ];

        let groups = filter_and_group_tags(&tags, "BEA");
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].key, 'B');
        assert_eq!(groups[0].tags.len(), 1);
        assert_eq!(groups[0].tags[0].name, "beach");
    }

    #[test]
    fn plain_click_selects_one_tag() {
        let update = update_selection(
            BTreeSet::from(["alpha".to_string(), "beta".to_string()]),
            &["alpha".into(), "beta".into(), "gamma".into()],
            Some("alpha".into()),
            "gamma",
            SelectionModifiers::default(),
        );
        assert_eq!(update.selected_tags, BTreeSet::from(["gamma".to_string()]));
        assert_eq!(update.anchor.as_deref(), Some("gamma"));
    }

    #[test]
    fn ctrl_click_toggles_additional_tags() {
        let update = update_selection(
            BTreeSet::from(["alpha".to_string()]),
            &["alpha".into(), "beta".into(), "gamma".into()],
            Some("alpha".into()),
            "beta",
            SelectionModifiers {
                ctrl: true,
                shift: false,
            },
        );
        assert_eq!(
            update.selected_tags,
            BTreeSet::from(["alpha".to_string(), "beta".to_string()])
        );
    }

    #[test]
    fn shift_click_extends_across_visible_order() {
        let update = update_selection(
            BTreeSet::from(["beta".to_string()]),
            &[
                "alpha".into(),
                "beta".into(),
                "gamma".into(),
                "delta".into(),
            ],
            Some("beta".into()),
            "delta",
            SelectionModifiers {
                ctrl: false,
                shift: true,
            },
        );
        assert_eq!(
            update.selected_tags,
            BTreeSet::from(["beta".to_string(), "gamma".to_string(), "delta".to_string()])
        );
    }
}
