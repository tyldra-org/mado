//! Memory pressure and the power source (spec 04 §7): the core sleeps tabs
//! under pressure, and energy saver halves the sleep thresholds on battery.

use std::ffi::c_void;
use std::time::Duration;

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2_foundation::{NSArray, NSDictionary, NSNumber, NSString};

/// How bad memory pressure is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pressure {
    Warning,
    Critical,
}

/// Where power comes from, and how full the battery is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PowerSource {
    pub on_battery: bool,
    /// 0 to 100; 100 without a battery.
    pub percent: u8,
}

type DispatchObject = *mut c_void;

const DISPATCH_MEMORYPRESSURE_WARN: usize = 0x02;
const DISPATCH_MEMORYPRESSURE_CRITICAL: usize = 0x04;

#[link(name = "System", kind = "dylib")]
unsafe extern "C" {
    static _dispatch_source_type_memorypressure: c_void;
    fn dispatch_get_global_queue(identifier: isize, flags: usize) -> DispatchObject;
    fn dispatch_source_create(
        kind: *const c_void,
        handle: usize,
        mask: usize,
        queue: DispatchObject,
    ) -> DispatchObject;
    fn dispatch_source_set_event_handler(source: DispatchObject, handler: &block2::Block<dyn Fn()>);
    fn dispatch_source_get_data(source: DispatchObject) -> usize;
    fn dispatch_resume(object: DispatchObject);
}

#[link(name = "IOKit", kind = "framework")]
unsafe extern "C" {
    fn IOPSCopyPowerSourcesInfo() -> *mut c_void;
    fn IOPSCopyPowerSourcesList(blob: *mut c_void) -> *mut c_void;
    fn IOPSGetPowerSourceDescription(blob: *mut c_void, source: *mut c_void) -> *mut c_void;
    fn IOPSGetProvidingPowerSourceType(snapshot: *mut c_void) -> *mut c_void;
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFRelease(object: *mut c_void);
}

/// Calls `report` on a background queue whenever macOS signals memory
/// pressure. Lives for the rest of the process.
pub fn watch_memory_pressure(report: impl Fn(Pressure) + Send + Sync + 'static) {
    // SAFETY: the type symbol is libdispatch's own; the global queue is
    // always valid; the source is never released, so the handler's pointer
    // to it stays valid.
    unsafe {
        let queue = dispatch_get_global_queue(0, 0);
        let source = dispatch_source_create(
            &raw const _dispatch_source_type_memorypressure,
            0,
            DISPATCH_MEMORYPRESSURE_WARN | DISPATCH_MEMORYPRESSURE_CRITICAL,
            queue,
        );
        if source.is_null() {
            return;
        }
        let address = source as usize;
        let handler = RcBlock::new(move || {
            // SAFETY: see above; the source outlives the handler.
            let data = dispatch_source_get_data(address as DispatchObject);
            let level = if data & DISPATCH_MEMORYPRESSURE_CRITICAL != 0 {
                Pressure::Critical
            } else {
                Pressure::Warning
            };
            report(level);
        });
        dispatch_source_set_event_handler(source, &handler);
        dispatch_resume(source);
    }
}

/// The current power source.
pub fn power_source() -> PowerSource {
    // SAFETY: IOKit's power source API. Every Copy result is released; the
    // Get results belong to the blob and are bridged without ownership.
    unsafe {
        let blob = IOPSCopyPowerSourcesInfo();
        if blob.is_null() {
            return PowerSource {
                on_battery: false,
                percent: 100,
            };
        }
        let providing = IOPSGetProvidingPowerSourceType(blob).cast::<NSString>();
        let on_battery = providing
            .as_ref()
            .is_some_and(|s| s.to_string() == "Battery Power");
        let mut percent = 100u8;
        let list_ref = IOPSCopyPowerSourcesList(blob);
        if let Some(list) = list_ref.cast::<NSArray<AnyObject>>().as_ref() {
            for item in list.iter() {
                let item: Retained<AnyObject> = item;
                let desc =
                    IOPSGetPowerSourceDescription(blob, Retained::as_ptr(&item).cast_mut().cast());
                let Some(desc) = desc.cast::<NSDictionary<NSString, AnyObject>>().as_ref() else {
                    continue;
                };
                let number = |key: &str| {
                    desc.objectForKey(&NSString::from_str(key))
                        .and_then(|v| v.downcast::<NSNumber>().ok())
                        .map(|n| n.integerValue())
                };
                if let (Some(now), Some(max)) = (number("Current Capacity"), number("Max Capacity"))
                    && max > 0
                {
                    percent = u8::try_from((now * 100 / max).clamp(0, 100)).unwrap_or(100);
                    break;
                }
            }
        }
        if !list_ref.is_null() {
            CFRelease(list_ref);
        }
        CFRelease(blob);
        PowerSource {
            on_battery,
            percent,
        }
    }
}

/// Polls the power source every `every` and calls `report` when it
/// changes. Runs on its own thread for the rest of the process.
pub fn watch_power(every: Duration, report: impl Fn(PowerSource) + Send + 'static) {
    let spawned = std::thread::Builder::new()
        .name("mado-power".into())
        .spawn(move || {
            let mut last = None;
            loop {
                let now = power_source();
                if last != Some(now) {
                    report(now);
                    last = Some(now);
                }
                std::thread::sleep(every);
            }
        });
    if let Err(e) = spawned {
        eprintln!("mado: the power watcher did not start: {e}");
    }
}

/// Calls `report` when the screen locks, the displays sleep, the Mac
/// sleeps or the session switches to another user (spec 11 §2.3: the
/// vault locks). Call once, on the main thread; the observers live for the
/// rest of the process and `report` runs on the main queue.
pub fn watch_screen_lock(report: impl Fn() + Send + Sync + 'static) {
    use objc2::msg_send;
    let report = std::sync::Arc::new(report);
    // SAFETY: the notification centers and the main queue are process-wide
    // objects; each observer copies its block, which only calls `report`.
    unsafe {
        let Ok(queue_class) = crate::objc::class(c"NSOperationQueue") else {
            return;
        };
        let main: Retained<AnyObject> = msg_send![queue_class, mainQueue];
        let observe = |center: &AnyObject, name: &str| {
            let r = std::sync::Arc::clone(&report);
            let block = RcBlock::new(move |_n: *mut AnyObject| r());
            let _token: Retained<AnyObject> = msg_send![
                center,
                addObserverForName: &*NSString::from_str(name),
                object: std::ptr::null::<AnyObject>(),
                queue: &*main,
                usingBlock: &*block
            ];
            // The token keeps the observer; it lives as long as the process.
            std::mem::forget(_token);
        };
        let workspace = objc2_app_kit::NSWorkspace::sharedWorkspace();
        let center = workspace.notificationCenter();
        for name in [
            "NSWorkspaceScreensDidSleepNotification",
            "NSWorkspaceWillSleepNotification",
            "NSWorkspaceSessionDidResignActiveNotification",
        ] {
            observe(&center, name);
        }
        if let Ok(dist) = crate::objc::class(c"NSDistributedNotificationCenter") {
            let center: Retained<AnyObject> = msg_send![dist, defaultCenter];
            observe(&center, "com.apple.screenIsLocked");
        }
    }
}

/// Calls `report` when the Mac wakes from sleep, the clock is set or the
/// time zone changes (spec 13 §12: routines' next runs are recomputed).
/// Call once, on the main thread; `report` runs on the main queue.
pub fn watch_wake_and_clock(report: impl Fn() + Send + Sync + 'static) {
    use objc2::msg_send;
    let report = std::sync::Arc::new(report);
    // SAFETY: as in `watch_screen_lock`.
    unsafe {
        let Ok(queue_class) = crate::objc::class(c"NSOperationQueue") else {
            return;
        };
        let main: Retained<AnyObject> = msg_send![queue_class, mainQueue];
        let observe = |center: &AnyObject, name: &str| {
            let r = std::sync::Arc::clone(&report);
            let block = RcBlock::new(move |_n: *mut AnyObject| r());
            let token: Retained<AnyObject> = msg_send![
                center,
                addObserverForName: &*NSString::from_str(name),
                object: std::ptr::null::<AnyObject>(),
                queue: &*main,
                usingBlock: &*block
            ];
            std::mem::forget(token);
        };
        let workspace = objc2_app_kit::NSWorkspace::sharedWorkspace();
        observe(
            &workspace.notificationCenter(),
            "NSWorkspaceDidWakeNotification",
        );
        if let Ok(nc) = crate::objc::class(c"NSNotificationCenter") {
            let center: Retained<AnyObject> = msg_send![nc, defaultCenter];
            observe(&center, "NSSystemClockDidChangeNotification");
            observe(&center, "NSSystemTimeZoneDidChangeNotification");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_power_source_reads_without_crashing() {
        let p = power_source();
        assert!(p.percent <= 100);
    }
}
