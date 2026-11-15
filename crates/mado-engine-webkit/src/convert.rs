//! Conversions between WebKit's Foundation types and the engine's.

use std::path::Path;

use mado_types::state::Rect;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2_app_kit::NSView;
use objc2_foundation::{
    NSData, NSDictionary, NSError, NSJSONSerialization, NSJSONWritingOptions, NSPoint, NSRect,
    NSSize, NSString, NSURL,
};
use serde_json::Value;

pub fn ns_url(url: &str) -> Option<Retained<NSURL>> {
    NSURL::URLWithString(&NSString::from_str(url))
}

pub fn url_string(url: Option<Retained<NSURL>>) -> String {
    url.and_then(|u| u.absoluteString())
        .map(|s| s.to_string())
        .unwrap_or_default()
}

/// `NSURLErrorDomain` codes keep their number; other domains report theirs.
pub fn error_code(error: &NSError) -> i64 {
    i64::try_from(error.code()).unwrap_or(i64::MIN)
}

/// A rectangle in the UI webview's CSS pixels, as a frame in the window's
/// content view (proto, M0 experiment 1): CSS (0, 0) sits below the title
/// bar's safe area, and the content view may count `y` from the bottom.
pub fn frame_in(content: &NSView, rect: Rect) -> NSRect {
    let Some(ui) = content.subviews().firstObject() else {
        return NSRect::new(
            NSPoint::new(rect.x, rect.y),
            NSSize::new(rect.width, rect.height),
        );
    };
    let top = ui.safeAreaInsets().top;
    let y = if ui.isFlipped() {
        rect.y + top
    } else {
        ui.bounds().size.height - top - rect.y - rect.height
    };
    let in_ui = NSRect::new(
        NSPoint::new(rect.x, y),
        NSSize::new(rect.width, rect.height),
    );
    ui.convertRect_toView(in_ui, Some(content))
}

pub fn data(bytes: &[u8]) -> Retained<NSData> {
    NSData::with_bytes(bytes)
}

pub fn bytes(data: &NSData) -> Vec<u8> {
    data.to_vec()
}

/// A Foundation value from JavaScript as JSON; anything JSON cannot hold
/// becomes its description.
pub fn to_json(value: Option<&AnyObject>) -> Value {
    let Some(value) = value else {
        return Value::Null;
    };
    if let Some(s) = value.downcast_ref::<NSString>() {
        return Value::String(s.to_string());
    }
    // A bare number or boolean is no JSON document for NSJSONSerialization.
    if let Some(n) = value.downcast_ref::<objc2_foundation::NSNumber>() {
        // SAFETY: `objCType` is a C string the number owns.
        let kind = unsafe { std::ffi::CStr::from_ptr(n.objCType().as_ptr()) }.to_bytes();
        let class = value.class().name().to_bytes();
        return if class == b"__NSCFBoolean" || kind == b"c" || kind == b"B" {
            Value::Bool(n.boolValue())
        } else if matches!(kind, b"f" | b"d") {
            serde_json::Number::from_f64(n.doubleValue()).map_or(Value::Null, Value::Number)
        } else {
            Value::from(n.longLongValue())
        };
    }
    // SAFETY: isValidJSONObject accepts any object.
    if unsafe { NSJSONSerialization::isValidJSONObject(value) } {
        // SAFETY: checked valid above.
        let data = unsafe {
            NSJSONSerialization::dataWithJSONObject_options_error(
                value,
                NSJSONWritingOptions::FragmentsAllowed,
            )
        };
        if let Ok(data) = data
            && let Ok(v) = serde_json::from_slice(&data.to_vec())
        {
            return v;
        }
    }
    // SAFETY: every Foundation object answers description.
    let text: Retained<NSString> = unsafe { objc2::msg_send![value, description] };
    Value::String(text.to_string())
}

/// A JSON value as a Foundation object for `callAsyncJavaScript`.
pub fn from_json(value: &Value) -> Option<Retained<AnyObject>> {
    let text = serde_json::to_vec(value).ok()?;
    let data = NSData::with_bytes(&text);
    NSJSONSerialization::JSONObjectWithData_options_error(
        &data,
        objc2_foundation::NSJSONReadingOptions::FragmentsAllowed,
    )
    .ok()
}

/// Safari's `CFBundleShortVersionString`, for the user agent (spec 02 §6).
pub fn safari_version() -> Option<String> {
    let path = NSString::from_str("/Applications/Safari.app/Contents/Info.plist");
    let url = NSURL::fileURLWithPath(&path);
    // SAFETY: reading a property list file.
    let plist =
        unsafe { NSDictionary::<NSString, AnyObject>::dictionaryWithContentsOfURL_error(&url) }
            .ok()?;
    let v = plist.objectForKey(&NSString::from_str("CFBundleShortVersionString"))?;
    let s = v.downcast_ref::<NSString>()?.to_string();
    let valid = !s.is_empty() && s.chars().all(|c| c.is_ascii_digit() || c == '.');
    valid.then_some(s)
}

pub fn write_file(path: &Path, data: &NSData) -> Result<(), String> {
    std::fs::write(path, data.to_vec()).map_err(|e| e.to_string())
}

/// macOS 27 and later ask WebKit's delegate about geolocation, so the
/// page-world shim is not needed (spec 07 §1.1, M0).
pub fn has_geolocation_delegate() -> bool {
    objc2_foundation::NSProcessInfo::processInfo()
        .operatingSystemVersion()
        .majorVersion
        >= 27
}
