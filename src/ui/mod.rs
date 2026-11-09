use egui::Color32;

pub const BG: Color32 = Color32::from_rgb(0x16, 0x18, 0x1d);
pub const PANEL: Color32 = Color32::from_rgb(0x22, 0x25, 0x2c);
pub const ACCENT: Color32 = Color32::from_rgb(0x4b, 0x8b, 0xe0);
pub const RECORD: Color32 = Color32::from_rgb(0xd9, 0x43, 0x4f);

/// Apply Oxbeat's dark look.
pub fn apply_theme(ctx: &egui::Context) {
    ctx.set_theme(egui::Theme::Dark);
    ctx.global_style_mut(|style| {
        let v = &mut style.visuals;
        v.dark_mode = true;
        v.panel_fill = PANEL;
        v.window_fill = PANEL;
        v.extreme_bg_color = BG;
        v.selection.bg_fill = ACCENT;
        v.hyperlink_color = ACCENT;
    });
}

pub struct OxbeatApp;

impl OxbeatApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        apply_theme(&cc.egui_ctx);
        Self
    }
}

impl eframe::App for OxbeatApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(BG))
            .show(ui, |_ui| {});
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn theme_is_dark() {
        let ctx = egui::Context::default();
        apply_theme(&ctx);
        assert!(ctx.global_style().visuals.dark_mode);
    }
}
