//! Asks for a TOTP code when a WeeChat relay has `relay.network.totp_secret` set.
//! The relay refuses the login and says a code is missing; this dialog takes one
//! and connects again with it.

use egui::{Key, Modifiers, RichText};

use crate::ui::app::WeeChatApp;

pub(crate) struct TotpPrompt {
    /// The connection asking.
    prefix: String,
    label: String,
    code: String,
    /// The last code was refused (wrong, or expired).
    invalid: bool,
    focused: bool,
}

impl WeeChatApp {
    /// The relay at `prefix` wants a code. A second refusal on the same
    /// connection keeps what's typed out and says the code was wrong.
    pub(crate) fn open_totp_prompt(&mut self, prefix: &str, invalid: bool) {
        let label = self.profiles.iter().find(|p| p.prefix() == prefix)
            .map(|p| p.label.clone())
            .unwrap_or_else(|| prefix.to_string());
        self.totp_prompt = Some(TotpPrompt {
            prefix: prefix.to_string(),
            label,
            code: String::new(),
            invalid,
            focused: false,
        });
        // The connections window is its own OS window and would sit on top of
        // this dialog, which lives in the main window.
        self.totp_raise = true;
    }

    pub(crate) fn show_totp_prompt(&mut self, ctx: &egui::Context) {
        let Some(prompt) = self.totp_prompt.as_mut() else { return };
        if std::mem::take(&mut self.totp_raise) {
            ctx.send_viewport_cmd_to(egui::ViewportId::ROOT, egui::ViewportCommand::Focus);
        }
        let (enter, escape) = ctx.input_mut(|i| (
            i.consume_key(Modifiers::NONE, Key::Enter),
            i.consume_key(Modifiers::NONE, Key::Escape),
        ));
        let mut cancel = escape;
        let mut submit = false;
        egui::Modal::new(egui::Id::unique("totp_prompt")).show(ctx, |ui| {
            ui.set_width(320.0);
            ui.label(RichText::new(format!("Two-factor code for {}", prompt.label)).strong());
            ui.add_space(4.0);
            let weak = ui.visuals().weak_text_color();
            ui.label(RichText::new("The relay asks for the 6-digit code from your authenticator app.").small().color(weak));
            ui.add_space(6.0);
            let edit = ui.add(
                egui::TextEdit::singleline(&mut prompt.code)
                    .char_limit(6)
                    .desired_width(f32::INFINITY)
                    .font(egui::TextStyle::Monospace)
                    .hint_text("123456"),
            );
            // Digits only; a pasted "123 456" still works.
            prompt.code.retain(|c| c.is_ascii_digit());
            if !prompt.focused {
                edit.request_focus();
                prompt.focused = true;
            }
            if prompt.invalid {
                ui.label(RichText::new("That code was refused. Wait for the next one and try again.")
                    .small().color(egui::Color32::from_rgb(220, 80, 80)));
            }
            ui.add_space(8.0);
            let ready = prompt.code.len() == 6;
            ui.horizontal(|ui| {
                if ui.add_enabled(ready, egui::Button::new("Connect")).clicked() {
                    submit = true;
                }
                if ui.button("Cancel").clicked() {
                    cancel = true;
                }
            });
            if enter && ready {
                submit = true;
            }
        });
        // A click outside the dialog doesn't dismiss it: that is how it got lost
        // behind the connections window. Cancel or Escape closes it.
        if cancel {
            let prompt = self.totp_prompt.take().expect("shown above");
            self.log_conn_for(&prompt.prefix, "TOTP code not entered, connection stopped");
        } else if submit {
            let prompt = self.totp_prompt.take().expect("shown above");
            let Some(profile) = self.profiles.iter().find(|p| p.prefix() == prompt.prefix).cloned() else { return };
            let password = self.session_passwords.get(&prompt.prefix).cloned().unwrap_or_default();
            self.pending_totp.insert(prompt.prefix, prompt.code);
            self.do_connect(&profile, password, ctx);
        }
    }
}
