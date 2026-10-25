//! Installed system fonts available for use in the interface.

use std::sync::{Arc, LazyLock};

use egui::FontFamily;

static FAMILIES: LazyLock<Vec<String>> = LazyLock::new(|| {
    let mut db = fontdb::Database::new();
    db.load_system_fonts();
    let mut names = db.faces().flat_map(|f| f.families.iter().map(|(n, _)| n.clone())).collect::<Vec<_>>();
    names.sort_by_key(|n| n.to_lowercase());
    names.dedup();
    names
});

pub fn families() -> &'static [String] {
    &FAMILIES
}

/// Install the selected system font as the first proportional and monospace face.
pub fn install(ctx: &egui::Context, family: &str) {
    let mut db = fontdb::Database::new();
    db.load_system_fonts();
    let fallback;
    let family = if family.is_empty() && plancraft_core::i18n::is_ja() {
        let found = ["Yu Gothic UI", "Yu Gothic", "Meiryo", "Noto Sans CJK JP", "Noto Sans JP", "MS Gothic"]
            .into_iter()
            .find(|candidate| FAMILIES.iter().any(|n| n.eq_ignore_ascii_case(candidate)));
        let Some(found) = found else { return };
        fallback = found;
        fallback
    } else if family.is_empty() {
        return;
    } else {
        family
    };
    let Some(id) = db.query(&fontdb::Query {
        families: &[fontdb::Family::Name(family)],
        weight: fontdb::Weight::NORMAL,
        stretch: fontdb::Stretch::Normal,
        style: fontdb::Style::Normal,
    }) else {
        return;
    };
    let Some(bytes) = db.with_face_data(id, |data, _| data.to_vec()) else {
        return;
    };
    let mut defs = ctx.fonts(|f| f.definitions().clone());
    defs.font_data.insert("user_selected".to_owned(), Arc::new(egui::FontData::from_owned(bytes)));
    defs.families.entry(FontFamily::Proportional).or_default().insert(0, "user_selected".to_owned());
    defs.families.entry(FontFamily::Monospace).or_default().insert(0, "user_selected".to_owned());
    ctx.set_fonts(defs);
}
