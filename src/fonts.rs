//! Font setup: a dedicated "terminal" family that prefers a user-chosen
//! monospace font (e.g. a Nerd Font) and falls back to egui's built-in Hack.

use std::sync::Arc;

use egui::{FontData, FontDefinitions, FontFamily};

pub const TERMINAL_FAMILY: &str = "terminal";

pub fn terminal_family() -> FontFamily {
    FontFamily::Name(TERMINAL_FAMILY.into())
}

pub fn install(ctx: &egui::Context, custom_path: &str) {
    let mut fonts = FontDefinitions::default();
    let mut term_list: Vec<String> = vec![];
    if !custom_path.trim().is_empty() {
        match std::fs::read(custom_path.trim()) {
            Ok(bytes) => {
                fonts
                    .font_data
                    .insert("custom-mono".into(), Arc::new(FontData::from_owned(bytes)));
                term_list.push("custom-mono".into());
            }
            Err(e) => log::warn!("cannot load font {custom_path}: {e}"),
        }
    }
    let mono = fonts
        .families
        .get(&FontFamily::Monospace)
        .cloned()
        .unwrap_or_default();
    term_list.extend(mono);
    // Symbols / emoji fallback from the proportional family.
    for f in fonts
        .families
        .get(&FontFamily::Proportional)
        .cloned()
        .unwrap_or_default()
    {
        if !term_list.contains(&f) {
            term_list.push(f);
        }
    }
    fonts.families.insert(terminal_family(), term_list);
    // Phosphor icons live in the private-use area, so they can sit behind any text font.
    fonts.font_data.insert(
        "phosphor".into(),
        Arc::new(FontData::from_static(include_bytes!(
            "../assets/fonts/Phosphor.ttf"
        ))),
    );
    for family in [FontFamily::Proportional, FontFamily::Monospace] {
        if let Some(list) = fonts.families.get_mut(&family) {
            list.push("phosphor".into());
        }
    }
    // Box-drawing, arrows and geometric symbols used in the UI only exist in Hack.
    if let Some(prop) = fonts.families.get_mut(&FontFamily::Proportional) {
        if !prop.iter().any(|f| f == "Hack") {
            prop.push("Hack".into());
        }
    }
    ctx.set_fonts(fonts);
}
