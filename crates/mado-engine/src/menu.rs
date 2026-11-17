//! The page context menu (spec 07 §8): which items show for what is under
//! the pointer, in the spec's section order. The adapter draws it; the core
//! runs the items the engine cannot.

use crate::event::{ContextAction, ContextData};

/// One line of the menu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MenuEntry {
    /// `label` is an `en.json` key; `{text}` in it is the selection.
    Item {
        action: ContextAction,
        label: &'static str,
    },
    Separator,
}

/// Facts about Mado the menu depends on.
#[derive(Debug, Clone, Default)]
pub struct MenuEnv {
    /// A default search engine is chosen.
    pub can_search: bool,
    /// AI page commands are on (`features.ai` and `features.textCommands`,
    /// spec 13 §6).
    pub ai: bool,
}

/// The page commands of the "Ask AI" section, for an editable field or
/// other selected text (spec 13 §6; the prompts live in mado-agent).
pub const AI_WRITING: [&str; 6] = [
    "improveWriting",
    "fixSpelling",
    "makeShorter",
    "makeFriendlier",
    "makeMoreDirect",
    "translate",
];
pub const AI_READING: [&str; 5] = [
    "summarize",
    "explain",
    "explainCode",
    "findBugs",
    "translate",
];

/// The menu label key of a page command.
pub fn ai_label(id: &str) -> &'static str {
    match id {
        "improveWriting" => "contextMenu.ai.improveWriting",
        "fixSpelling" => "contextMenu.ai.fixSpelling",
        "makeShorter" => "contextMenu.ai.makeShorter",
        "makeFriendlier" => "contextMenu.ai.makeFriendlier",
        "makeMoreDirect" => "contextMenu.ai.makeMoreDirect",
        "translate" => "contextMenu.ai.translate",
        "summarize" => "contextMenu.ai.summarize",
        "explain" => "contextMenu.ai.explain",
        "explainCode" => "contextMenu.ai.explainCode",
        _ => "contextMenu.ai.findBugs",
    }
}

/// Who runs an action: the engine (edit commands, media, saving through
/// the engine, navigation in the page, Inspect) or the core.
pub fn engine_runs(action: &ContextAction) -> bool {
    use ContextAction as A;
    matches!(
        action,
        A::SaveLinkAs
            | A::SaveImageAs
            | A::CopyImage
            | A::MediaPlayPause
            | A::MediaMute
            | A::MediaLoop
            | A::MediaShowControls
            | A::MediaPictureInPicture
            | A::SaveVideoAs
            | A::Copy
            | A::LookUp
            | A::Undo
            | A::Redo
            | A::Cut
            | A::Paste
            | A::PasteAndMatchStyle
            | A::SelectAll
            | A::Spelling(_)
            | A::EmojiAndSymbols
            | A::Back
            | A::Forward
            | A::Reload
            | A::Inspect
    )
}

/// The menu for `c`. Items whose feature does not exist yet are left out
/// (private windows and profiles: M2-T09; Reader: M2-T08; view source and
/// page Markdown: M2-T14; Save as with the File menu: M2-T15; AI and Fill:
/// M5, M4).
pub fn build(c: &ContextData, env: &MenuEnv) -> Vec<MenuEntry> {
    use ContextAction as A;
    let mut sections: Vec<Vec<(A, &'static str)>> = Vec::new();
    if c.link_url.is_some() {
        sections.push(vec![
            (A::OpenLinkInNewTab, "contextMenu.openLinkInNewTab"),
            (A::OpenLinkInNewWindow, "contextMenu.openLinkInNewWindow"),
            (A::OpenLinkInSafari, "contextMenu.openLinkInSafari"),
            (A::SaveLinkAs, "contextMenu.saveLinkAs"),
            (A::CopyLink, "contextMenu.copyLink"),
            (A::CopyCleanLink, "contextMenu.copyCleanLink"),
            (A::CopyLinkAsMarkdown, "contextMenu.copyLinkAsMarkdown"),
        ]);
    }
    if c.image_url.is_some() {
        sections.push(vec![
            (A::OpenImageInNewTab, "contextMenu.openImageInNewTab"),
            (A::SaveImageAs, "contextMenu.saveImageAs"),
            (A::CopyImage, "contextMenu.copyImage"),
            (A::CopyImageAddress, "contextMenu.copyImageAddress"),
        ]);
    }
    if c.media_url.is_some() {
        let mut media = vec![
            (A::MediaPlayPause, "contextMenu.mediaPlayPause"),
            (A::MediaMute, "contextMenu.mediaMute"),
            (A::MediaLoop, "contextMenu.mediaLoop"),
            (A::MediaShowControls, "contextMenu.mediaShowControls"),
        ];
        if c.media_kind == Some(crate::event::MediaKind::Video) {
            media.push((
                A::MediaPictureInPicture,
                "contextMenu.mediaPictureInPicture",
            ));
            media.push((A::SaveVideoAs, "contextMenu.saveVideoAs"));
        }
        sections.push(media);
    }
    let selected = c.selection.as_deref().is_some_and(|s| !s.trim().is_empty());
    if c.editable {
        let mut edit: Vec<(A, &'static str)> = c
            .spelling_suggestions
            .iter()
            .take(5)
            .map(|s| (A::Spelling(s.clone()), "contextMenu.spelling"))
            .collect();
        edit.extend([
            (A::Undo, "contextMenu.undo"),
            (A::Redo, "contextMenu.redo"),
            (A::Cut, "contextMenu.cut"),
            (A::Copy, "contextMenu.copy"),
            (A::Paste, "contextMenu.paste"),
            (A::PasteAndMatchStyle, "contextMenu.pasteAndMatchStyle"),
            (A::SelectAll, "contextMenu.selectAll"),
            (A::EmojiAndSymbols, "contextMenu.emojiAndSymbols"),
        ]);
        sections.push(edit);
    } else if selected {
        let mut sel = vec![(A::Copy, "contextMenu.copy")];
        if env.can_search {
            sel.push((A::SearchSelection, "contextMenu.searchSelection"));
        }
        sel.push((A::LookUp, "contextMenu.lookUp"));
        sel.push((A::CopyLinkToHighlight, "contextMenu.copyLinkToHighlight"));
        sections.push(sel);
    }
    if env.ai && selected {
        let ids: &[&str] = if c.editable { &AI_WRITING } else { &AI_READING };
        sections.push(
            ids.iter()
                .map(|id| (A::AskAi((*id).to_owned()), ai_label(id)))
                .collect(),
        );
    }
    if sections.is_empty() {
        sections.push(vec![
            (A::Back, "contextMenu.back"),
            (A::Forward, "contextMenu.forward"),
            (A::Reload, "contextMenu.reload"),
            (A::Print, "contextMenu.print"),
        ]);
    }
    sections.push(vec![(A::Inspect, "contextMenu.inspect")]);
    let mut out = Vec::new();
    for (i, s) in sections.into_iter().enumerate() {
        if i > 0 {
            out.push(MenuEntry::Separator);
        }
        out.extend(
            s.into_iter()
                .map(|(action, label)| MenuEntry::Item { action, label }),
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn labels(m: &[MenuEntry]) -> Vec<&'static str> {
        m.iter()
            .map(|e| match e {
                MenuEntry::Item { label, .. } => label.trim_start_matches("contextMenu."),
                MenuEntry::Separator => "-",
            })
            .collect()
    }

    #[test]
    fn sections_follow_the_spec_order() {
        let link_and_image = ContextData {
            link_url: Some("https://a.example/".into()),
            image_url: Some("https://a.example/i.png".into()),
            ..ContextData::default()
        };
        let m = labels(&build(&link_and_image, &MenuEnv::default()));
        assert_eq!(m.first(), Some(&"openLinkInNewTab"));
        let sep = m.iter().position(|l| *l == "-").expect("a separator");
        assert_eq!(m[sep + 1], "openImageInNewTab");
        assert_eq!(m.last(), Some(&"inspect"));
        let page = labels(&build(&ContextData::default(), &MenuEnv::default()));
        assert_eq!(
            page,
            vec!["back", "forward", "reload", "print", "-", "inspect"]
        );
        let sel = ContextData {
            selection: Some("rust".into()),
            ..ContextData::default()
        };
        let with_engine = labels(&build(
            &sel,
            &MenuEnv {
                can_search: true,
                ai: false,
            },
        ));
        assert!(with_engine.contains(&"searchSelection"));
        assert!(!labels(&build(&sel, &MenuEnv::default())).contains(&"searchSelection"));
        let ai = MenuEnv {
            can_search: false,
            ai: true,
        };
        assert!(labels(&build(&sel, &ai)).contains(&"ai.summarize"));
        let field = ContextData {
            selection: Some("teh".into()),
            editable: true,
            ..ContextData::default()
        };
        let m = labels(&build(&field, &ai));
        assert!(m.contains(&"ai.improveWriting") && !m.contains(&"ai.summarize"));
        assert!(
            !labels(&build(&ContextData::default(), &ai))
                .iter()
                .any(|l| l.starts_with("ai."))
        );
    }
}
