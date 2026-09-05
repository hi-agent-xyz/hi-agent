//! Adding a core: an address and a pairing code, which is the whole of
//! attachment. There is no QR scanner here — a desktop has a keyboard, and the
//! phones' scanners exist because they do not.

use std::rc::Rc;

use adw::prelude::*;
use gtk::glib::clone;

use crate::core::models::CoreError;
use crate::model::AppModel;
use crate::paths::log;

/// An `AdwDialog` rather than an `AdwAlertDialog`, for one reason: an alert
/// closes itself the moment a response is activated, and a rejected pairing
/// code has to leave the address and the code on screen to be corrected.
pub fn present(parent: &impl IsA<gtk::Widget>, model: Rc<AppModel>) {
    let address = adw::EntryRow::builder().title("Address").build();
    let code = adw::EntryRow::builder().title("Pairing code").build();
    let label = adw::EntryRow::builder().title("Call it").build();

    let group = adw::PreferencesGroup::builder()
        .description(
            "A core's address is a URL — http://localhost:12358 for one on this machine, \
             or https://hi-agent.xyz/name for one you reach from anywhere. The pairing \
             code comes from that core.",
        )
        .build();
    group.add(&address);
    group.add(&code);
    group.add(&label);

    let error = gtk::Label::builder()
        .wrap(true)
        .visible(false)
        .xalign(0.0)
        .build();
    error.add_css_class("error");

    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(12)
        .margin_top(12)
        .margin_bottom(18)
        .margin_start(18)
        .margin_end(18)
        .build();
    content.append(&group);
    content.append(&error);

    let cancel = gtk::Button::with_label("Cancel");
    let add = gtk::Button::with_label("Add");
    add.add_css_class("suggested-action");
    let spinner = adw::Spinner::builder()
        .width_request(18)
        .height_request(18)
        .visible(false)
        .build();

    let header = adw::HeaderBar::builder()
        .show_end_title_buttons(false)
        .show_start_title_buttons(false)
        .title_widget(&adw::WindowTitle::new("Add a core", ""))
        .build();
    header.pack_start(&cancel);
    header.pack_end(&add);
    header.pack_end(&spinner);

    let layout = adw::ToolbarView::builder().content(&content).build();
    layout.add_top_bar(&header);

    let dialog = adw::Dialog::builder()
        .title("Add a core")
        .content_width(520)
        .child(&layout)
        .build();

    cancel.connect_clicked(clone!(
        #[weak]
        dialog,
        move |_| { dialog.close(); }
    ));

    add.connect_clicked(clone!(
        #[strong] model,
        #[strong] address,
        #[strong] code,
        #[strong] label,
        #[strong] error,
        #[strong] spinner,
        #[weak] dialog,
        move |add| {
            let (address, code, label) = (
                address.text().to_string(),
                code.text().to_string(),
                label.text().to_string(),
            );
            add.set_sensitive(false);
            spinner.set_visible(true);
            error.set_visible(false);
            glib::spawn_future_local(clone!(
                #[strong] model,
                #[strong] error,
                #[strong] spinner,
                #[strong] add,
                #[weak] dialog,
                async move {
                    let outcome = model.add_core(&address, &code, &label).await;
                    spinner.set_visible(false);
                    add.set_sensitive(true);
                    match outcome {
                        Ok(()) => { dialog.close(); }
                        Err(e) => {
                            // An address the person mistyped is not worth a log
                            // line; anything the core said is.
                            if !matches!(e, CoreError::InvalidAddress(_)) {
                                log(format!("add core: {e}"));
                            }
                            error.set_text(&e.to_string());
                            error.set_visible(true);
                        }
                    }
                }
            ));
        }
    ));

    dialog.present(Some(parent));
}
