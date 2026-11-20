use rfd::{MessageButtons, MessageDialog, MessageLevel};

pub(crate) fn show_error(title: &str, message: &str) {
    log::error!("{title}: {message}");

    MessageDialog::new()
        .set_level(MessageLevel::Error)
        .set_title(title)
        .set_description(message)
        .set_buttons(MessageButtons::Ok)
        .show();
}
