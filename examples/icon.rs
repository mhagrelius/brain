//! Render the app icons to PNG, to see what they actually look like.
//!
//! ```sh
//! cargo run --example icon -- /tmp/icons
//! ```

use gtk::prelude::*;

fn main() {
    let out = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "/tmp/icons".to_string());
    gtk::init().expect("a display — run under xvfb-run if there is none");
    std::fs::create_dir_all(&out).expect("output directory");

    for (path, size) in [
        ("data/icons/hicolor/scalable/apps/us.hagreli.Brain.svg", 128),
        (
            "data/icons/hicolor/symbolic/apps/us.hagreli.Brain-symbolic.svg",
            64,
        ),
    ] {
        let name = std::path::Path::new(path)
            .file_stem()
            .map(|stem| stem.to_string_lossy().to_string())
            .unwrap_or_default();

        let picture = gtk::Picture::for_filename(path);
        let Some(paintable) = picture.paintable() else {
            eprintln!("{path}: did not load");
            continue;
        };

        let snapshot = gtk::Snapshot::new();
        paintable.snapshot(&snapshot, size as f64, size as f64);
        let Some(node) = snapshot.to_node() else {
            eprintln!("{path}: drew nothing");
            continue;
        };

        let renderer = gtk::gsk::CairoRenderer::new();
        renderer
            .realize(gtk::gdk::Surface::NONE)
            .expect("a renderer");
        let texture = renderer.render_texture(&node, None);
        let target = format!("{out}/{name}.png");
        texture.save_to_png(&target).expect("write the png");
        renderer.unrealize();
        println!("wrote {target}");
    }
}
