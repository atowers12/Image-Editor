//! Shared application artwork used by the native window and in-app branding.

/// The supplied logo remains the single source of truth for every icon use.
pub const LOGO_JPEG: &[u8] = include_bytes!("../LOGO 2.jpg");

pub fn icon_data(size: u32) -> Option<egui::IconData> {
    let image = image::load_from_memory(LOGO_JPEG)
        .ok()?
        .resize_exact(size, size, image::imageops::FilterType::Lanczos3)
        .to_rgba8();
    Some(egui::IconData {
        rgba: image.into_raw(),
        width: size,
        height: size,
    })
}
