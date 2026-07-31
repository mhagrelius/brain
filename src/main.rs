use brain::ui::BrainApplication;
use gtk::prelude::*;

fn main() -> gtk::glib::ExitCode {
    gtk::glib::set_application_name("Brain");
    gtk::glib::set_prgname(Some(brain::APP_ID));
    BrainApplication::new().run()
}
