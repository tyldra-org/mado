//! Store extensions an import found in another browser (spec 09 §11,
//! 12 §5): Settings › Extensions offers each for reinstall through the
//! store install and its prompt until it is installed or dismissed.

use std::collections::BTreeMap;

use mado_types::ExtensionId;
use mado_types::command::Command;
use mado_types::patch::PatchRecorder;
use mado_types::state::{ImportedExtension, Notice, NoticeAction};
use serde_json::json;

use super::Core;

/// The `imported_extensions` rows that are not installed.
pub(super) fn load(conn: &rusqlite::Connection) -> rusqlite::Result<Vec<ImportedExtension>> {
    let mut stmt = conn.prepare(
        "SELECT id, name, browser FROM imported_extensions
         WHERE id NOT IN (SELECT id FROM extensions) ORDER BY imported_at, name",
    )?;
    let rows = stmt
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get(1)?, r.get(2)?)))?
        .filter_map(Result::ok)
        .filter_map(|(id, name, browser)| {
            Some(ImportedExtension {
                id: id.parse().ok()?,
                name,
                browser,
            })
        })
        .collect();
    Ok(rows)
}

impl Core {
    /// Reads the offered extensions again, after startup or an import.
    pub(super) fn refresh_imported(&mut self, rec: &mut PatchRecorder) {
        if self.extensions.is_none() || self.state.profile.is_private {
            return;
        }
        let imported = self.profile_db.readers().read(load).unwrap_or_else(|e| {
            tracing::error!(error = %e, "the imported extensions could not be read");
            Vec::new()
        });
        if imported != self.state.extensions.imported {
            let mut ext = self.state.extensions.clone();
            ext.imported = imported;
            self.state.tx(rec).set_extensions(ext);
        }
    }

    /// After an import that found extensions: a notice that opens
    /// Settings › Extensions, where they are offered.
    pub(super) fn offer_imported(&mut self, browser: &str, rec: &mut PatchRecorder) {
        self.refresh_imported(rec);
        let count = self.state.extensions.imported.len();
        if count == 0 {
            return;
        }
        let id = format!("n{}", self.next_notice);
        self.next_notice += 1;
        let notice = Notice {
            id,
            message_key: "notice.importedExtensions".into(),
            args: BTreeMap::from([
                ("browser".to_owned(), json!(browser)),
                ("count".to_owned(), json!(count)),
            ]),
            action: Some(NoticeAction {
                label_key: "notice.reviewExtensions".into(),
                command: Command::AppOpenSettings {
                    section: Some("extensions".into()),
                },
            }),
            undo: None,
            expires_at: None,
        };
        self.add_notice(notice, rec);
    }

    /// Forgets imported extension `id`, or all of them: installed, or
    /// dismissed by the user.
    pub(super) fn forget_imported(&mut self, id: Option<&ExtensionId>, rec: &mut PatchRecorder) {
        let mut ext = self.state.extensions.clone();
        let before = ext.imported.len();
        ext.imported.retain(|e| id.is_some_and(|id| &e.id != id));
        if ext.imported.len() != before {
            self.state.tx(rec).set_extensions(ext);
        }
        let key = id.map(ToString::to_string);
        let pending = self.profile_db.writer().write(move |c| {
            match &key {
                Some(k) => c.execute("DELETE FROM imported_extensions WHERE id = ?1", [k]),
                None => c.execute("DELETE FROM imported_extensions", []),
            }
            .map(|_| ())
        });
        tokio::task::spawn_local(async move {
            if let Err(e) = pending.await {
                tracing::error!(error = %e, "an imported extension was not forgotten");
            }
        });
    }
}
