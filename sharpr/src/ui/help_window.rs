use gtk4::prelude::*;
use gtk4::{gio, glib};
use libadwaita as adw;
use libadwaita::prelude::*;

pub fn show_help_window(parent: &impl gtk4::prelude::IsA<gtk4::Widget>) {
    let bytes = gio::resources_lookup_data(
        "/io/github/hebbihebb/Sharpr/manual.md",
        gio::ResourceLookupFlags::NONE,
    )
    .expect("manual.md not bundled in GResource");
    let text = std::str::from_utf8(bytes.as_ref()).unwrap_or("");

    let dialog = adw::Dialog::new();
    dialog.set_title("Sharpr Manual");
    dialog.set_content_width(620);
    dialog.set_content_height(700);

    let toolbar_view = adw::ToolbarView::new();
    let header = adw::HeaderBar::new();
    toolbar_view.add_top_bar(&header);

    let content = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    content.set_margin_top(24);
    content.set_margin_bottom(32);
    content.set_margin_start(28);
    content.set_margin_end(28);

    render_markdown(text, &content);

    let scroll = gtk4::ScrolledWindow::new();
    scroll.set_hexpand(true);
    scroll.set_vexpand(true);
    scroll.set_hscrollbar_policy(gtk4::PolicyType::Never);
    scroll.set_vscrollbar_policy(gtk4::PolicyType::Automatic);
    scroll.set_child(Some(&content));

    toolbar_view.set_content(Some(&scroll));
    dialog.set_child(Some(&toolbar_view));
    dialog.present(Some(parent));
}

fn parse_link(line: &str) -> Option<(String, String)> {
    // Matches: [label](url) with nothing else on the line
    let line = line.trim();
    if !line.starts_with('[') {
        return None;
    }
    let close_bracket = line.find(']')?;
    let label = line[1..close_bracket].to_string();
    let rest = &line[close_bracket + 1..];
    if !rest.starts_with('(') || !rest.ends_with(')') {
        return None;
    }
    let url = rest[1..rest.len() - 1].to_string();
    Some((label, url))
}

fn inline_markup(text: &str) -> String {
    let escaped = text
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");
    let bold = replace_delimited(&escaped, "**", "<b>", "</b>");
    replace_delimited(&bold, "*", "<i>", "</i>")
}

fn replace_delimited(input: &str, delim: &str, open: &str, close: &str) -> String {
    let mut result = String::with_capacity(input.len());
    let mut rest = input;
    loop {
        match rest.find(delim) {
            None => {
                result.push_str(rest);
                break;
            }
            Some(start) => {
                result.push_str(&rest[..start]);
                let after_open = &rest[start + delim.len()..];
                match after_open.find(delim) {
                    None => {
                        result.push_str(delim);
                        rest = after_open;
                    }
                    Some(end) => {
                        result.push_str(open);
                        result.push_str(&after_open[..end]);
                        result.push_str(close);
                        rest = &after_open[end + delim.len()..];
                    }
                }
            }
        }
    }
    result
}

fn add_label(
    container: &gtk4::Box,
    markup: &str,
    css_classes: &[&str],
    margin_top: i32,
    margin_bottom: i32,
) {
    let label = gtk4::Label::new(None);
    label.set_markup(markup);
    label.set_halign(gtk4::Align::Start);
    label.set_xalign(0.0);
    label.set_wrap(true);
    label.set_wrap_mode(gtk4::pango::WrapMode::WordChar);
    label.set_margin_top(margin_top);
    label.set_margin_bottom(margin_bottom);
    for cls in css_classes {
        label.add_css_class(cls);
    }
    container.append(&label);
}

fn flush_table(rows: &mut Vec<Vec<String>>, container: &gtk4::Box) {
    if rows.is_empty() {
        return;
    }
    let grid = gtk4::Grid::new();
    grid.set_halign(gtk4::Align::Start);
    grid.set_margin_top(8);
    grid.set_margin_bottom(8);
    grid.set_column_spacing(24);
    grid.set_row_spacing(3);

    for (row_idx, cells) in rows.iter().enumerate() {
        for (col_idx, cell) in cells.iter().enumerate() {
            let label = gtk4::Label::new(None);
            let markup = if row_idx == 0 {
                format!("<b>{}</b>", glib::markup_escape_text(cell))
            } else {
                glib::markup_escape_text(cell).to_string()
            };
            label.set_markup(&markup);
            label.set_halign(gtk4::Align::Start);
            label.set_xalign(0.0);
            grid.attach(&label, col_idx as i32, row_idx as i32, 1, 1);
        }
    }

    container.append(&grid);
    rows.clear();
}

fn flush_paragraph(buf: &mut Vec<String>, container: &gtk4::Box) {
    if buf.is_empty() {
        return;
    }
    let text = buf.join(" ");
    buf.clear();
    if text.trim().is_empty() {
        return;
    }
    let markup = inline_markup(&text);
    add_label(container, &markup, &[], 0, 6);
}

fn render_markdown(text: &str, container: &gtk4::Box) {
    let mut para_buf: Vec<String> = Vec::new();
    let mut table_buf: Vec<Vec<String>> = Vec::new();
    let mut first_heading = true;

    for line in text.lines() {
        // Table rows
        if line.trim_start().starts_with('|') {
            flush_paragraph(&mut para_buf, container);
            if line.contains("---") {
                // separator — marks end of header row, skip
                continue;
            }
            let cells: Vec<String> = line
                .trim()
                .trim_matches('|')
                .split('|')
                .map(|s| s.trim().to_string())
                .collect();
            table_buf.push(cells);
            continue;
        }

        // Flush pending table when we leave a table block
        if !table_buf.is_empty() {
            flush_table(&mut table_buf, container);
        }

        if let Some(rest) = line.strip_prefix("### ") {
            flush_paragraph(&mut para_buf, container);
            let markup = format!("<b>{}</b>", glib::markup_escape_text(rest));
            add_label(container, &markup, &[], 12, 4);
            continue;
        }
        if let Some(rest) = line.strip_prefix("## ") {
            flush_paragraph(&mut para_buf, container);
            let top = if first_heading { 0 } else { 20 };
            let markup = format!("<b><big>{}</big></b>", glib::markup_escape_text(rest));
            add_label(container, &markup, &["title-4"], top, 4);
            first_heading = false;
            continue;
        }
        if let Some(rest) = line.strip_prefix("# ") {
            flush_paragraph(&mut para_buf, container);
            let markup = format!("<b>{}</b>", glib::markup_escape_text(rest));
            add_label(container, &markup, &["title-2"], 0, 8);
            first_heading = false;
            continue;
        }

        // Standalone link: [label](url)
        if line.trim_start().starts_with('[') {
            if let Some((label, url)) = parse_link(line.trim()) {
                flush_paragraph(&mut para_buf, container);
                let btn = gtk4::LinkButton::with_label(&url, &label);
                btn.set_halign(gtk4::Align::Start);
                btn.set_margin_top(2);
                btn.set_margin_bottom(2);
                container.append(&btn);
                continue;
            }
        }

        if let Some(rest) = line.strip_prefix("- ").or_else(|| line.strip_prefix("* ")) {
            flush_paragraph(&mut para_buf, container);
            let markup = format!("• {}", inline_markup(rest));
            add_label(container, &markup, &[], 2, 2);
            continue;
        }

        if line.trim().is_empty() {
            flush_paragraph(&mut para_buf, container);
            continue;
        }

        para_buf.push(line.to_string());
    }

    if !table_buf.is_empty() {
        flush_table(&mut table_buf, container);
    }
    flush_paragraph(&mut para_buf, container);
}
