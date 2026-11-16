//! The blocker's content rule lists (spec 10 §4): looked up or compiled
//! in the shared store, then swapped onto every tab's user content
//! controller. A tab on a host where the blocker is off carries none; the
//! check runs as each main-frame navigation is decided, before it loads.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use block2::RcBlock;
use mado_engine::{BlockerRules, RuleSource};
use mado_types::TabId;
use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2_foundation::{NSArray, NSError, NSString, NSURL};
use objc2_web_kit::{WKContentRuleList, WKContentRuleListStore, WKUserContentController};

#[derive(Default)]
struct State {
    /// Bumped by each `set_blocker_rules`; late compiles of an older set
    /// are dropped.
    generation: u64,
    active: Vec<Retained<WKContentRuleList>>,
    off_hosts: Vec<String>,
    /// The lists each tab carries now.
    carried: BTreeMap<TabId, Vec<Retained<WKContentRuleList>>>,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
}

fn host_of(url: &str) -> Option<String> {
    let rest = url.split_once("://")?.1;
    let authority = rest.split(['/', '?', '#']).next()?;
    let host = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    let host = host.split(':').next()?;
    (!host.is_empty()).then(|| host.to_ascii_lowercase())
}

/// Gives `controller` exactly `lists`.
fn swap(
    controller: &WKUserContentController,
    from: &[Retained<WKContentRuleList>],
    to: &[Retained<WKContentRuleList>],
) {
    // SAFETY: a live controller and compiled lists, on the main thread.
    unsafe {
        for l in from {
            controller.removeContentRuleList(l);
        }
        for l in to {
            controller.addContentRuleList(l);
        }
    }
}

/// What a page at `url` should carry.
fn wanted(s: &State, url: Option<&str>) -> Vec<Retained<WKContentRuleList>> {
    let off = url
        .and_then(host_of)
        .is_some_and(|h| mado_engine::host_in(&h, &s.off_hosts));
    if off { Vec::new() } else { s.active.clone() }
}

fn same(a: &[Retained<WKContentRuleList>], b: &[Retained<WKContentRuleList>]) -> bool {
    a.len() == b.len()
        && a.iter()
            .zip(b)
            .all(|(x, y)| Retained::as_ptr(x) == Retained::as_ptr(y))
}

/// A new tab view's controller: the active lists.
pub(crate) fn attach(tab: TabId, controller: &WKUserContentController) {
    STATE.with_borrow_mut(|s| {
        let lists = s.active.clone();
        swap(controller, &[], &lists);
        s.carried.insert(tab, lists);
    });
}

pub(crate) fn forget(tab: TabId) {
    STATE.with_borrow_mut(|s| s.carried.remove(&tab));
}

/// A main-frame navigation to `url` was decided: the tab's lists follow
/// the site switch of its new host.
pub(crate) fn navigating(tab: TabId, url: &str) {
    let Some(controller) = crate::with(|a| a.controller(tab)).flatten() else {
        return;
    };
    STATE.with_borrow_mut(|s| {
        let want = wanted(s, Some(url));
        let have = s.carried.get(&tab).cloned().unwrap_or_default();
        if !same(&have, &want) {
            swap(&controller, &have, &want);
            s.carried.insert(tab, want);
        }
    });
}

/// Every tab gets the new lists, by its current URL.
fn activate(lists: Vec<Retained<WKContentRuleList>>, off_hosts: Vec<String>) {
    let tabs: Vec<(TabId, Retained<WKUserContentController>, Option<String>)> =
        crate::with(|a| a.blocker_tabs()).unwrap_or_default();
    let count = u32::try_from(lists.len()).unwrap_or(u32::MAX);
    STATE.with_borrow_mut(|s| {
        s.active = lists;
        s.off_hosts = off_hosts;
        for (tab, controller, url) in tabs {
            let want = wanted(s, url.as_deref());
            let have = s.carried.get(&tab).cloned().unwrap_or_default();
            if !same(&have, &want) {
                swap(&controller, &have, &want);
            }
            s.carried.insert(tab, want);
        }
    });
    crate::delegate::send(mado_engine::EngineEvent::BlockerRulesApplied { lists: count });
}

fn error_text(e: *mut NSError) -> String {
    // SAFETY: WebKit hands a valid error or null.
    unsafe { e.as_ref() }.map_or_else(
        || "no error".into(),
        |e| e.localizedDescription().to_string(),
    )
}

/// Applies `rules` (spec 10 §4 steps 3 and 5).
pub fn set_blocker_rules(rules: BlockerRules) {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let generation = STATE.with_borrow_mut(|s| {
        s.generation += 1;
        s.generation
    });
    if rules.lists.is_empty() {
        activate(Vec::new(), rules.off_hosts);
        return;
    }
    if let Err(e) = std::fs::create_dir_all(&rules.store) {
        tracing::error!(error = %e, "the rule list store folder");
        return;
    }
    let url = NSURL::fileURLWithPath_isDirectory(
        &NSString::from_str(&rules.store.to_string_lossy()),
        true,
    );
    // SAFETY: a store at a folder Mado owns, on the main thread.
    let Some(store) = (unsafe { WKContentRuleListStore::storeWithURL(Some(&url), mtm) }) else {
        tracing::error!("WebKit opened no rule list store");
        return;
    };
    let n = rules.lists.len();
    let results: Rc<RefCell<Vec<Option<Retained<WKContentRuleList>>>>> =
        Rc::new(RefCell::new(vec![None; n]));
    let remaining = Rc::new(RefCell::new(n));
    let off = Rc::new(rules.off_hosts.clone());
    let wanted_ids: Rc<Vec<String>> =
        Rc::new(rules.lists.iter().map(|l| l.identifier.clone()).collect());
    for (i, list) in rules.lists.into_iter().enumerate() {
        let finish = {
            let (results, remaining, off, ids, store) = (
                Rc::clone(&results),
                Rc::clone(&remaining),
                Rc::clone(&off),
                Rc::clone(&wanted_ids),
                store.clone(),
            );
            move |compiled: Option<Retained<WKContentRuleList>>| {
                results.borrow_mut()[i] = compiled;
                *remaining.borrow_mut() -= 1;
                if *remaining.borrow() > 0 || STATE.with_borrow(|s| s.generation) != generation {
                    return;
                }
                let lists: Vec<_> = results.borrow_mut().drain(..).flatten().collect();
                tracing::info!(lists = lists.len(), "blocker rules applied");
                activate(lists, (*off).clone());
                prune(&store, &ids);
            }
        };
        let finish = Rc::new(finish);
        let identifier = NSString::from_str(&list.identifier);
        let compile = {
            let (store, finish, identifier, name) = (
                store.clone(),
                Rc::clone(&finish),
                identifier.clone(),
                list.identifier.clone(),
            );
            move || {
                let json = match &list.source {
                    RuleSource::Json(j) => Some(j.clone()),
                    RuleSource::File(p) => std::fs::read_to_string(p)
                        .inspect_err(|e| tracing::error!(list = %name, error = %e, "a rule file is unreadable"))
                        .ok(),
                };
                let Some(json) = json else {
                    finish(None);
                    return;
                };
                let finish = Rc::clone(&finish);
                let name = name.clone();
                let done = RcBlock::new(move |l: *mut WKContentRuleList, e: *mut NSError| {
                    // SAFETY: WebKit hands a list or an error.
                    let list = unsafe { Retained::retain(l) };
                    if list.is_none() {
                        tracing::error!(list = %name, error = %error_text(e), "a rule list did not compile");
                    }
                    finish(list);
                });
                // SAFETY: strings and a block WebKit calls once, on the main thread.
                unsafe {
                    store.compileContentRuleListForIdentifier_encodedContentRuleList_completionHandler(
                        Some(&identifier),
                        Some(&NSString::from_str(&json)),
                        Some(&done),
                    );
                }
            }
        };
        let compile = RefCell::new(Some(compile));
        let looked = RcBlock::new(move |l: *mut WKContentRuleList, _e: *mut NSError| {
            // SAFETY: WebKit hands a list or null.
            match unsafe { Retained::retain(l) } {
                Some(list) => finish(Some(list)),
                None => {
                    if let Some(c) = compile.borrow_mut().take() {
                        c();
                    }
                }
            }
        });
        // SAFETY: as above.
        unsafe {
            store.lookUpContentRuleListForIdentifier_completionHandler(
                Some(&identifier),
                Some(&looked),
            )
        };
    }
}

/// Shared lists of older builds and this profile's older own lists leave
/// the store; compiled lists in use stay loaded.
fn prune(store: &WKContentRuleListStore, wanted: &[String]) {
    let own: Vec<String> = wanted
        .iter()
        .filter_map(|w| {
            w.starts_with("mado-site-")
                .then(|| w.rsplit_once('-').map(|(p, _)| format!("{p}-")))
                .flatten()
        })
        .collect();
    let wanted = wanted.to_vec();
    let store_for_remove = objc2::Message::retain(store);
    let block = RcBlock::new(move |ids: *mut NSArray<NSString>| {
        // SAFETY: WebKit hands an array or null.
        let Some(ids) = (unsafe { ids.as_ref() }) else {
            return;
        };
        for id in ids.iter() {
            let id = id.to_string();
            let shared = id.strip_prefix("mado-").is_some_and(|r| {
                r.len() > 33
                    && r.as_bytes()[32] == b'-'
                    && r[..32].bytes().all(|b| b.is_ascii_hexdigit())
            });
            let stale = !wanted.contains(&id) && (shared || own.iter().any(|p| id.starts_with(p)));
            if stale {
                let done = RcBlock::new(|_e: *mut NSError| {});
                // SAFETY: removing a list by identifier, on the main thread.
                unsafe {
                    store_for_remove.removeContentRuleListForIdentifier_completionHandler(
                        Some(&NSString::from_str(&id)),
                        Some(&done),
                    );
                }
            }
        }
    });
    // SAFETY: as above.
    unsafe { store.getAvailableContentRuleListIdentifiers(Some(&block)) };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hosts_come_from_urls() {
        assert_eq!(
            host_of("https://a.Example.com:8443/x?y").as_deref(),
            Some("a.example.com")
        );
        assert_eq!(
            host_of("http://u:p@b.example/").as_deref(),
            Some("b.example")
        );
        assert_eq!(host_of("about:blank"), None);
    }
}
