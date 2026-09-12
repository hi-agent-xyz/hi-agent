//! Adding a remote agent, name-first.
//!
//! **The name leads because it is the only path that suits this machine.** There
//! is no QR scanner here and there never was — a desktop has a keyboard, and the
//! phones' scanners exist because they do not. But a keyboard was never the
//! problem: reading a one-time code off *another* screen means walking to it, and
//! a headless box in a rack has no other screen to walk to. Asking needs a name
//! and somebody to say yes.
//!
//! The address-and-code form is still here, folded away, for a self-hosted core
//! with no name in the default zone.

use std::rc::Rc;

use adw::prelude::*;
use gtk::glib::clone;

use crate::core::client;
use crate::core::models::CoreError;
use crate::model::AppModel;
use crate::paths::log;

/// An `AdwDialog` rather than an `AdwAlertDialog`, for one reason: an alert closes
/// itself the moment a response is activated, and a refused name has to leave what
/// was typed on screen to be corrected.
pub fn present(parent: &impl IsA<gtk::Widget>, model: Rc<AppModel>) {
    // — the name, which is the whole of the ordinary path
    let name = adw::EntryRow::builder().title("Your agent's name").build();
    // The zone rides in the row's own suffix slot, so typing `iloahz` reads as
    // `iloahz.hi-agent.xyz` and no sentence has to be spent explaining what a name
    // is.
    let zone = gtk::Label::builder()
        .label(format!(".{}", client::DEFAULT_ZONE))
        .build();
    zone.add_css_class("dim-label");
    name.add_suffix(&zone);
    // Only while what is typed is still a bare label, which is the only case
    // `address_for_name` appends the zone in. A row reading
    // `http://localhost:12358.hi-agent.xyz` describes a request nobody is about to
    // make.
    name.connect_changed(clone!(
        #[weak]
        zone,
        move |entry| {
            let typed = entry.text().to_string();
            let bare = typed.trim().trim_start_matches('@');
            zone.set_visible(!bare.contains('.') && !bare.contains('/'));
        }
    ));

    let naming = adw::PreferencesGroup::builder()
        .description(
            "Say which agent this is. It will ask to be let in, and you approve it \
             there — nothing long to type here.",
        )
        .build();
    naming.add(&name);

    // — the fallback, for a self-hosted core
    let address = adw::EntryRow::builder().title("Address").build();
    let code = adw::EntryRow::builder().title("One-time code").build();
    let label = adw::EntryRow::builder().title("Call it").build();

    let manual = adw::PreferencesGroup::builder()
        .description(
            "For an agent with no name in the zone: its address — \
             http://localhost:12358 for one on this machine — and a one-time code \
             it is showing.",
        )
        .build();
    manual.add(&address);
    manual.add(&code);
    manual.add(&label);

    let expander = gtk::Expander::builder()
        .label("Use a full address")
        .child(&manual)
        .build();

    // — the wait
    let shown_code = gtk::Label::builder().visible(false).build();
    shown_code.add_css_class("title-1");
    shown_code.add_css_class("numeric");
    let waiting_for = gtk::Label::builder().wrap(true).visible(false).xalign(0.5).build();
    waiting_for.add_css_class("dim-label");

    let error = gtk::Label::builder().wrap(true).visible(false).xalign(0.0).build();
    error.add_css_class("error");

    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(12)
        .margin_top(12)
        .margin_bottom(18)
        .margin_start(18)
        .margin_end(18)
        .build();
    content.append(&naming);
    content.append(&shown_code);
    content.append(&waiting_for);
    content.append(&expander);
    content.append(&error);

    let cancel = gtk::Button::with_label("Cancel");
    let ask = gtk::Button::with_label("Ask to be let in");
    ask.add_css_class("suggested-action");
    let spinner = adw::Spinner::builder()
        .width_request(18)
        .height_request(18)
        .visible(false)
        .build();

    let header = adw::HeaderBar::builder()
        .show_end_title_buttons(false)
        .show_start_title_buttons(false)
        .title_widget(&adw::WindowTitle::new("Add a remote hi-agent", ""))
        .build();
    header.pack_start(&cancel);
    header.pack_end(&ask);
    header.pack_end(&spinner);

    let layout = adw::ToolbarView::builder().content(&content).build();
    layout.add_top_bar(&header);

    let dialog = adw::Dialog::builder()
        .title("Add a remote hi-agent")
        .content_width(520)
        .child(&layout)
        .build();

    cancel.connect_clicked(clone!(
        #[weak]
        dialog,
        move |_| { dialog.close(); }
    ));

    // The header button does whichever of the two paths the person is using: the
    // name when the fallback is folded away, the address and code when it is open.
    // One button rather than two, because only one of them is ever the thing being
    // filled in.
    ask.connect_clicked(clone!(
        #[strong] model,
        #[strong] name,
        #[strong] address,
        #[strong] code,
        #[strong] label,
        #[strong] error,
        #[strong] spinner,
        #[strong] expander,
        #[strong] naming,
        #[strong] shown_code,
        #[strong] waiting_for,
        #[weak] dialog,
        move |ask| {
            let using_address = expander.is_expanded() && !address.text().trim().is_empty();
            let (name_text, address_text, code_text, label_text) = (
                name.text().to_string(),
                address.text().to_string(),
                code.text().to_string(),
                label.text().to_string(),
            );
            ask.set_sensitive(false);
            spinner.set_visible(true);
            error.set_visible(false);

            glib::spawn_future_local(clone!(
                #[strong] model,
                #[strong] error,
                #[strong] spinner,
                #[strong] ask,
                #[strong] expander,
                #[strong] naming,
                #[strong] shown_code,
                #[strong] waiting_for,
                #[weak] dialog,
                async move {
                    let outcome = if using_address {
                        model.add_core(&address_text, &code_text, &label_text).await
                    } else {
                        match model.ask_to_join(&name_text).await {
                            Ok(invitation) => {
                                // Nothing for this person to do now but read the
                                // number out, so the form goes away and the number
                                // is the screen.
                                naming.set_visible(false);
                                expander.set_visible(false);
                                shown_code.set_text(&invitation.code);
                                shown_code.set_visible(true);
                                waiting_for.set_text(&format!(
                                    "Open Reach on {} and let this machine in. \
                                     Check the code matches.",
                                    invitation.label
                                ));
                                waiting_for.set_visible(true);
                                ask.set_visible(false);
                                model.join(&invitation).await
                            }
                            Err(e) => Err(e),
                        }
                    };

                    spinner.set_visible(false);
                    ask.set_sensitive(true);
                    match outcome {
                        Ok(()) => { dialog.close(); }
                        Err(e) => {
                            // A name the person mistyped is not worth a log line;
                            // anything the agent said is.
                            if !matches!(
                                e,
                                CoreError::InvalidAddress(_) | CoreError::InvalidName
                            ) {
                                log(format!("add agent: {e}"));
                            }
                            // Back to the form, with what went wrong under it.
                            naming.set_visible(true);
                            expander.set_visible(true);
                            shown_code.set_visible(false);
                            waiting_for.set_visible(false);
                            ask.set_visible(true);
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
