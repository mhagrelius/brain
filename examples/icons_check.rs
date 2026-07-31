//! Check that every icon name the app uses resolves in the installed theme.
//!
//! A bare `icon_name` that does not resolve draws a "missing image" glyph, and
//! nothing warns about it — so this asserts, rather than trusting memory.

const USED: &[&str] = &[
    "accessories-text-editor-symbolic",
    "document-new-symbolic",
    "edit-clear-symbolic",
    "image-missing-symbolic",
    "insert-link-symbolic",
    "mail-attachment-symbolic",
    "open-menu-symbolic",
    "sidebar-show-symbolic",
    "system-search-symbolic",
    "user-bookmarks-symbolic",
    "view-list-symbolic",
    "sidebar-show-right-symbolic",
    "document-edit-symbolic",
    "tag-symbolic",
    // Candidates the HIG names, checked so a swap is informed.
    "list-add-symbolic",
    "user-trash-symbolic",
    "document-edit-symbolic",
    "view-refresh-symbolic",
    "chain-link-symbolic",
    "text-editor-symbolic",
    "dock-left-symbolic",
    "view-sidebar-start-symbolic",
    "edit-find-symbolic",
    "attach-symbolic",
    "tag-symbolic",
    "view-list-bullet-symbolic",
];

fn main() {
    gtk::init().expect("a display — run under xvfb-run if there is none");
    let display = gtk::gdk::Display::default().expect("a display");
    let theme = gtk::IconTheme::for_display(&display);

    let mut missing = Vec::new();
    for name in USED {
        if theme.has_icon(name) {
            println!("ok      {name}");
        } else {
            println!("MISSING {name}");
            missing.push(*name);
        }
    }
    if !missing.is_empty() {
        eprintln!(
            "\n{} icon(s) do not resolve: {}",
            missing.len(),
            missing.join(", ")
        );
    }
}
