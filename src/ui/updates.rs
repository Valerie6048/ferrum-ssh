use super::{password_input, primary_button};
use crate::app::{App, Tab};
use crate::updates::{RELEASES_URL, State};

impl App {
    pub fn update_dialog(&mut self, ui: &mut egui::Ui) -> bool {
        let mut close = false;
        let pal = self.pal();
        ui.label(egui::RichText::new("Software updates").size(20.0).strong());
        ui.label(format!("Installed version: {}", env!("CARGO_PKG_VERSION")));
        ui.add_space(12.0);
        ui.add_enabled_ui(!self.updater.busy(), |ui| {
            if ui.checkbox(&mut self.updater.previews, "Include preview releases").changed() {
                self.updater.state = State::Idle;
            }
            egui::CollapsingHeader::new("Private repository access").show(ui, |ui| {
                ui.label("Uses your GitHub CLI sign-in, or a token entered below. The token stays in memory for this session.");
                ui.label("GitHub token");
                password_input(ui, &mut self.updater.token, "Optional when signed in with gh", f32::INFINITY);
            });
        });
        ui.add_space(12.0);
        let mut download = false;
        let mut restart = false;
        match &self.updater.state {
            State::Idle => {
                ui.label("Check GitHub for a newer release.");
            }
            State::Checking => {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label("Checking GitHub releases…");
                });
            }
            State::Current(message) => {
                ui.label(message);
            }
            State::Failed(message) => {
                ui.colored_label(pal.danger, "Update could not complete");
                ui.label(message);
            }
            State::Available(release) => {
                ui.label(
                    egui::RichText::new(format!("Ferrum {} is available", release.tag_name))
                        .strong(),
                );
                egui::ScrollArea::vertical()
                    .max_height(160.0)
                    .show(ui, |ui| {
                        ui.label(
                            release
                                .body
                                .as_deref()
                                .unwrap_or("No release notes provided."),
                        );
                    });
                ui.add_space(8.0);
                download = ui.add(primary_button("Download update", pal)).clicked();
                ui.label(egui::RichText::new("The download is checked against GitHub’s SHA-256 digest before installation.").small().color(pal.text_dim));
            }
            State::Downloading(done, total) => {
                let fraction = if *total == 0 {
                    0.0
                } else {
                    *done as f32 / *total as f32
                };
                ui.add(
                    egui::ProgressBar::new(fraction)
                        .show_percentage()
                        .animate(true),
                );
                ui.label(format!(
                    "Downloaded {:.1} of {:.1} MB",
                    *done as f64 / 1_000_000.0,
                    *total as f64 / 1_000_000.0
                ));
            }
            State::Ready(prepared) => {
                ui.label(
                    egui::RichText::new(format!("Ferrum {} is ready to install", prepared.version))
                        .color(pal.success),
                );
                ui.label("The package passed its integrity checks. Ferrum will save your vault, install the update, and reopen.");
                let sessions = self
                    .tabs
                    .iter()
                    .filter_map(|tab| match tab {
                        Tab::Terminal(t) => Some(t),
                        _ => None,
                    })
                    .flat_map(|t| &t.panes)
                    .filter(|p| p.session.is_live())
                    .count();
                if sessions > 0 {
                    ui.colored_label(
                        pal.warning,
                        format!("Restarting will disconnect {sessions} active terminal sessions."),
                    );
                }
                if self.editor.is_some() {
                    ui.colored_label(
                        pal.warning,
                        "Save any changes in the open form before restarting.",
                    );
                }
                restart = ui
                    .add_enabled(
                        self.qa.is_none(),
                        primary_button("Install and restart", pal),
                    )
                    .clicked();
            }
        }
        if download {
            self.updater.download(self.rt.handle(), ui.ctx());
        }
        if restart {
            let saved = !self.loaded
                || self
                    .store
                    .save(&self.vault)
                    .map_err(|e| self.toast(format!("Cannot save the vault: {e}"), true))
                    .is_ok();
            if saved {
                match self.updater.install() {
                    Ok(()) => ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close),
                    Err(e) => self.toast(format!("Cannot install update: {e:#}"), true),
                }
            }
        }
        ui.add_space(12.0);
        ui.separator();
        ui.horizontal(|ui| {
            if ui
                .add_enabled(!self.updater.busy(), egui::Button::new("Check again"))
                .clicked()
            {
                self.updater.check(self.rt.handle(), ui.ctx());
            }
            ui.hyperlink_to("GitHub releases", RELEASES_URL);
            if ui.button("Close").clicked() {
                close = true;
            }
        });
        close
    }
}
