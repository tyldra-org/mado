//! Applying the recorded patches to the old document gives the new one
//! (M1-T03, Done when). Random edits go through the generated setters; the
//! recorded operations are applied with an independent RFC 6902
//! implementation (`json-patch`) to the JSON of the state before the edits.

use mado_types::state::{
    AudioState, DownloadState, DownloadStatus, GroupColor, GroupState, State, TabLife, TabState,
    WindowState,
};
use mado_types::{DownloadId, GroupId, PatchRecorder, TabId, WindowId};
use proptest::prelude::*;
use serde_json::{Value, json};

#[derive(Debug, Clone)]
enum Edit {
    NewWindow,
    NewTab { window: usize, title: String },
    CloseTab(usize),
    SetTitle { tab: usize, title: String },
    SetLife { tab: usize, loading: bool },
    SetProgress { tab: usize, progress: f64 },
    Mute { tab: usize, muted: bool },
    Activate { tab: usize },
    Sidebar { window: usize, width: f64 },
    Group { tab: usize, color: u8 },
    Ungroup(usize),
    Download { bytes: u64 },
    Setting { key: u8, value: i64 },
    Feature { key: u8, on: bool },
    ProfileName(String),
}

fn edit() -> impl Strategy<Value = Edit> {
    let text = "[a-z~/ ]{0,8}";
    prop_oneof![
        Just(Edit::NewWindow),
        (any::<usize>(), text).prop_map(|(window, title)| Edit::NewTab { window, title }),
        any::<usize>().prop_map(Edit::CloseTab),
        (any::<usize>(), text).prop_map(|(tab, title)| Edit::SetTitle { tab, title }),
        (any::<usize>(), any::<bool>()).prop_map(|(tab, loading)| Edit::SetLife { tab, loading }),
        (any::<usize>(), 0.0..1.0f64)
            .prop_map(|(tab, progress)| Edit::SetProgress { tab, progress }),
        (any::<usize>(), any::<bool>()).prop_map(|(tab, muted)| Edit::Mute { tab, muted }),
        any::<usize>().prop_map(|tab| Edit::Activate { tab }),
        (any::<usize>(), 120.0..480.0f64)
            .prop_map(|(window, width)| Edit::Sidebar { window, width }),
        (any::<usize>(), 0u8..9).prop_map(|(tab, color)| Edit::Group { tab, color }),
        any::<usize>().prop_map(Edit::Ungroup),
        any::<u64>().prop_map(|bytes| Edit::Download { bytes }),
        (0u8..6, any::<i64>()).prop_map(|(key, value)| Edit::Setting { key, value }),
        (0u8..4, any::<bool>()).prop_map(|(key, on)| Edit::Feature { key, on }),
        text.prop_map(Edit::ProfileName),
    ]
}

const COLORS: [GroupColor; 9] = [
    GroupColor::Grey,
    GroupColor::Blue,
    GroupColor::Red,
    GroupColor::Yellow,
    GroupColor::Green,
    GroupColor::Pink,
    GroupColor::Purple,
    GroupColor::Cyan,
    GroupColor::Orange,
];

fn pick<K: Clone>(keys: &[K], i: usize) -> Option<K> {
    (!keys.is_empty()).then(|| keys[i % keys.len()].clone())
}

/// Applies one edit through the generated setters.
fn apply(state: &mut State, rec: &mut PatchRecorder, edit: Edit) {
    let windows: Vec<WindowId> = state.windows.keys().copied().collect();
    let tabs: Vec<TabId> = state.tabs.keys().copied().collect();
    let mut tx = state.tx(rec);
    match edit {
        Edit::NewWindow => {
            let id = WindowId::new();
            tx.windows().insert(
                id,
                WindowState {
                    id,
                    ..WindowState::default()
                },
            );
        }
        Edit::NewTab { window, title } => {
            let Some(window) = pick(&windows, window) else {
                return;
            };
            let id = TabId::new();
            tx.tabs().insert(
                id,
                TabState {
                    id,
                    window_id: window,
                    title,
                    zoom: 1.0,
                    ..TabState::default()
                },
            );
            if let Some(mut w) = tx.windows().get(&window) {
                let mut order = w.get().tab_order.clone();
                order.push(id);
                w.set_tab_order(order);
            }
        }
        Edit::CloseTab(i) => {
            let Some(id) = pick(&tabs, i) else { return };
            let Some(tab) = tx.tabs().remove(&id) else {
                return;
            };
            if let Some(mut w) = tx.windows().get(&tab.window_id) {
                let order: Vec<TabId> = w
                    .get()
                    .tab_order
                    .iter()
                    .copied()
                    .filter(|t| *t != id)
                    .collect();
                w.set_tab_order(order);
                if w.get().active_tab_id == Some(id) {
                    w.set_active_tab_id(None);
                }
            }
        }
        Edit::SetTitle { tab, title } => {
            if let Some(mut t) = pick(&tabs, tab).and_then(|id| tx.tabs().get(&id)) {
                t.set_title(title);
            }
        }
        Edit::SetLife { tab, loading } => {
            if let Some(mut t) = pick(&tabs, tab).and_then(|id| tx.tabs().get(&id)) {
                t.set_life(if loading {
                    TabLife::Loading
                } else {
                    TabLife::Live
                });
            }
        }
        Edit::SetProgress { tab, progress } => {
            if let Some(mut t) = pick(&tabs, tab).and_then(|id| tx.tabs().get(&id)) {
                t.set_progress(progress);
                t.set_progress(progress / 2.0);
            }
        }
        Edit::Mute { tab, muted } => {
            if let Some(mut t) = pick(&tabs, tab).and_then(|id| tx.tabs().get(&id)) {
                t.audio().set_muted(muted);
                if muted {
                    t.set_audio(AudioState {
                        playing: false,
                        muted,
                    });
                }
            }
        }
        Edit::Activate { tab } => {
            let Some(id) = pick(&tabs, tab) else { return };
            let Some(window) = tx.get_tab_window(&id) else {
                return;
            };
            if let Some(mut w) = tx.windows().get(&window) {
                w.set_active_tab_id(Some(id));
            }
        }
        Edit::Sidebar { window, width } => {
            if let Some(mut w) = pick(&windows, window).and_then(|id| tx.windows().get(&id)) {
                w.sidebar().set_width(width);
            }
        }
        Edit::Group { tab, color } => {
            let Some(id) = pick(&tabs, tab) else { return };
            let group = GroupId::new();
            tx.groups().insert(
                group,
                GroupState {
                    id: group,
                    color: COLORS[usize::from(color)],
                    ..GroupState::default()
                },
            );
            if let Some(mut t) = tx.tabs().get(&id) {
                t.set_group_id(Some(group));
            }
        }
        Edit::Ungroup(i) => {
            let groups: Vec<GroupId> = tx.get().groups.keys().copied().collect();
            if let Some(g) = pick(&groups, i) {
                tx.groups().remove(&g);
            }
        }
        Edit::Download { bytes } => {
            let id = DownloadId::new();
            tx.downloads().insert(
                id,
                DownloadState {
                    id,
                    state: DownloadStatus::InProgress,
                    ..DownloadState::default()
                },
            );
            if let Some(mut d) = tx.downloads().get(&id) {
                d.set_received(bytes);
                d.set_state(DownloadStatus::Complete);
            }
        }
        Edit::Setting { key, value } => {
            let key = format!("section.key{key}");
            tx.settings().values().insert(key.clone(), json!(value));
            let mut changed = tx.get().settings.changed.clone();
            if !changed.contains(&key) {
                changed.push(key);
                tx.settings().set_changed(changed);
            }
        }
        Edit::Feature { key, on } => {
            tx.features().insert(format!("feature{key}"), on);
        }
        Edit::ProfileName(name) => {
            tx.profile().set_name(name);
        }
    }
}

trait TabWindow {
    fn get_tab_window(&self, tab: &TabId) -> Option<WindowId>;
}

impl TabWindow for mado_types::state::StatePatch<'_> {
    fn get_tab_window(&self, tab: &TabId) -> Option<WindowId> {
        self.get().tabs.get(tab).map(|t| t.window_id)
    }
}

fn check(edits: Vec<Edit>) -> Result<(), TestCaseError> {
    let mut state = State::default();
    let mut rec = PatchRecorder::new();
    // A few revisions, as the core emits one per command batch.
    for batch in edits.chunks(7) {
        let before: Value =
            serde_json::to_value(&state).map_err(|e| TestCaseError::fail(e.to_string()))?;
        for e in batch.iter().cloned() {
            apply(&mut state, &mut rec, e);
        }
        let ops =
            serde_json::to_value(rec.take()).map_err(|e| TestCaseError::fail(e.to_string()))?;
        let patch: json_patch::Patch =
            serde_json::from_value(ops).map_err(|e| TestCaseError::fail(e.to_string()))?;
        let mut mirrored = before;
        json_patch::patch(&mut mirrored, &patch).map_err(|e| TestCaseError::fail(e.to_string()))?;
        let after = serde_json::to_value(&state).map_err(|e| TestCaseError::fail(e.to_string()))?;
        prop_assert_eq!(mirrored, after);
    }
    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(400))]

    #[test]
    fn recorded_patches_reproduce_the_mutated_state(edits in prop::collection::vec(edit(), 1..120)) {
        check(edits)?;
    }
}

#[test]
fn unchanged_values_record_nothing() {
    let mut state = State::default();
    let mut rec = PatchRecorder::new();
    let name = state.profile.name.clone();
    state.tx(&mut rec).profile().set_name(name);
    assert!(rec.is_empty());
}

#[test]
fn map_keys_are_escaped_in_paths() {
    let mut state = State::default();
    let mut rec = PatchRecorder::new();
    state.tx(&mut rec).features().insert("a/b~c".into(), true);
    assert_eq!(rec.ops()[0].path(), "/features/a~1b~0c");
}
