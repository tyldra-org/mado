//! Frame handles (spec 02 §5.1, 11 §4.2): the `WKFrameInfo` of a frame
//! that sent a page message, kept so the core can post back to that frame
//! alone (the fill values go to the frame whose fields they are). At most
//! 64 per tab; the oldest go first.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, VecDeque};

use mado_types::TabId;
use objc2::rc::Retained;
use objc2_web_kit::WKFrameInfo;

const PER_TAB: usize = 64;

type Handles = HashMap<TabId, VecDeque<(u64, Retained<WKFrameInfo>)>>;

thread_local! {
    static FRAMES: RefCell<Handles> = RefCell::new(HashMap::new());
    static NEXT: Cell<u64> = const { Cell::new(1) };
}

/// Keeps `frame` for `tab` and returns its new handle.
pub(crate) fn remember(tab: TabId, frame: Retained<WKFrameInfo>) -> u64 {
    let id = NEXT.get();
    NEXT.set(id + 1);
    FRAMES.with_borrow_mut(|f| {
        let list = f.entry(tab).or_default();
        if list.len() >= PER_TAB {
            list.pop_front();
        }
        list.push_back((id, frame));
    });
    id
}

pub(crate) fn get(tab: TabId, handle: u64) -> Option<Retained<WKFrameInfo>> {
    FRAMES.with_borrow(|f| {
        f.get(&tab)?
            .iter()
            .find(|(id, _)| *id == handle)
            .map(|(_, frame)| frame.clone())
    })
}

pub(crate) fn forget(tab: TabId) {
    FRAMES.with_borrow_mut(|f| f.remove(&tab));
}
