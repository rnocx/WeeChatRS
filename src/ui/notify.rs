//! Cross-platform desktop notifications using direct OS APIs (Option A).
//!
//! - **Linux**: zbus → `org.freedesktop.Notifications.Notify` with embedded
//!   PNG passed as `image-data` hint. No subprocess, no fallback CLI.
//! - **macOS**: `mac-notification-sys` directly (no notify-rust wrapper). Uses
//!   the bundle's icon automatically; we do not pass an icon path so the OS
//!   doesn't try to "open" it via LaunchServices (which previously triggered
//!   the "Choose Application" picker).
//! - **Windows**: `notify-rust` is still used (kept for now; native Win32 toast
//!   support can be added later).
//!
//! Every call spawns a short-lived OS thread so we never block the UI loop, and
//! so zbus on Linux gets a clean stack outside of any tokio runtime context
//! (zbus's blocking proxy calls `block_on` which panics if invoked inside one).

#[allow(dead_code)]
pub struct Notification {
    pub app_name: String,
    pub title: String,
    pub body: String,
}

/// Fire the notification on a background thread. Errors are logged via `log::warn!`
/// and otherwise swallowed — a failed notification must never crash the app.
pub fn show(notif: Notification) {
    std::thread::spawn(move || {
        if let Err(e) = backend::deliver(&notif) {
            log::warn!("desktop notification failed: {}", e);
        }
    });
}

/// Call once at startup. Currently only used on macOS to register the bundle
/// identifier with `mac-notification-sys` so it doesn't fall back to its
/// auto-discovery path (which triggers the LaunchServices picker on some
/// macOS setups).
pub fn init() {
    backend::init();
}

#[cfg(target_os = "linux")]
mod backend {
    use super::Notification;
    use std::collections::HashMap;
    use zbus::blocking::Connection;
    use zbus::zvariant::{Structure, Value};

    pub fn init() {}

    /// Embedded application icon, decoded once and cached as a structured
    /// `image-data` payload ready to hand to the FDO Notifications service.
    fn icon_image_data() -> Option<(i32, i32, i32, bool, i32, i32, Vec<u8>)> {
        use std::sync::OnceLock;
        static CACHED: OnceLock<Option<(i32, i32, i32, bool, i32, i32, Vec<u8>)>> = OnceLock::new();
        CACHED.get_or_init(|| {
            let bytes: &[u8] = include_bytes!("../../assets/icon.png");
            let img = image::load_from_memory(bytes).ok()?.to_rgba8();
            let (w, h) = (img.width() as i32, img.height() as i32);
            let rowstride = w * 4;
            let data = img.into_raw();
            // (width, height, rowstride, has_alpha, bits_per_sample, channels, data)
            Some((w, h, rowstride, true, 8, 4, data))
        }).clone()
    }

    pub fn deliver(notif: &Notification) -> Result<(), String> {
        let conn = Connection::session().map_err(|e| format!("dbus session: {}", e))?;

        let mut hints: HashMap<&str, Value> = HashMap::new();

        // image-data: (iiibii ay) per the Desktop Notifications spec
        if let Some((w, h, rs, alpha, bps, ch, data)) = icon_image_data() {
            let s = Structure::from((w, h, rs, alpha, bps, ch, data));
            hints.insert("image-data", Value::Structure(s));
        }

        // Helps notification daemons categorise / pick a sound.
        hints.insert("category", Value::Str("im.received".into()));
        hints.insert("urgency", Value::U8(1)); // 0=low, 1=normal, 2=critical
        hints.insert("desktop-entry", Value::Str("weechatrs".into()));

        let actions: Vec<&str> = Vec::new();
        let app_icon: &str = ""; // covered by image-data hint
        let replaces_id: u32 = 0;
        let expire_timeout: i32 = -1; // server default

        conn.call_method(
            Some("org.freedesktop.Notifications"),
            "/org/freedesktop/Notifications",
            Some("org.freedesktop.Notifications"),
            "Notify",
            &(
                notif.app_name.as_str(),
                replaces_id,
                app_icon,
                notif.title.as_str(),
                notif.body.as_str(),
                actions,
                hints,
                expire_timeout,
            ),
        ).map_err(|e| format!("Notify call: {}", e))?;
        Ok(())
    }
}

#[cfg(target_os = "macos")]
mod backend {
    use super::Notification;
    use block2::RcBlock;
    use objc2::rc::Retained;
    use objc2::runtime::{Bool, ProtocolObject};
    use objc2::{define_class, msg_send, AnyThread};
    use objc2_foundation::{NSError, NSObject, NSObjectProtocol, NSString};
    use objc2_user_notifications::{
        UNAuthorizationOptions, UNMutableNotificationContent,
        UNNotification, UNNotificationPresentationOptions,
        UNNotificationRequest, UNUserNotificationCenter,
        UNUserNotificationCenterDelegate,
    };

    /// Returns true only when running inside a proper `.app` bundle.
    /// `UNUserNotificationCenter::currentNotificationCenter()` crashes
    /// (bundleProxyForCurrentProcess is nil) when there is no bundle.
    fn in_app_bundle() -> bool {
        std::env::current_exe()
            .map(|p| p.to_string_lossy().contains(".app/Contents/MacOS/"))
            .unwrap_or(false)
    }

    // Delegate that tells macOS to show banners even when the app is in the
    // foreground. Without this the OS only bounces the dock icon.
    define_class!(
        #[unsafe(super(NSObject))]
        #[name = "WeeChatRSNotifDelegate"]
        struct NotifDelegate;

        unsafe impl NSObjectProtocol for NotifDelegate {}

        unsafe impl UNUserNotificationCenterDelegate for NotifDelegate {
            #[unsafe(method(userNotificationCenter:willPresentNotification:withCompletionHandler:))]
            fn will_present(
                &self,
                _center: &UNUserNotificationCenter,
                _notification: &UNNotification,
                completion_handler: &block2::DynBlock<dyn Fn(UNNotificationPresentationOptions)>,
            ) {
                completion_handler.call((
                    UNNotificationPresentationOptions::Banner
                        | UNNotificationPresentationOptions::Sound,
                ));
            }
        }
    );

    impl NotifDelegate {
        fn new() -> Retained<Self> {
            let this = Self::alloc();
            unsafe { msg_send![this, init] }
        }
    }

    pub fn init() {
        if !in_app_bundle() {
            return;
        }

        let center = UNUserNotificationCenter::currentNotificationCenter();

        // Install delegate so foreground notifications show as banners.
        // Leaked intentionally — delegate must live for the process lifetime.
        let delegate = NotifDelegate::new();
        let delegate_obj = ProtocolObject::from_retained(delegate);
        center.setDelegate(Some(&*delegate_obj));
        std::mem::forget(delegate_obj);

        // Request permission (macOS shows the prompt only once; subsequent
        // calls are no-ops if already authorized or denied).
        let options = UNAuthorizationOptions::Alert | UNAuthorizationOptions::Sound;
        let handler = RcBlock::new(|granted: Bool, error: *mut NSError| {
            if !granted.as_bool() {
                log::warn!("macOS notification permission not granted");
            }
            if !error.is_null() {
                log::warn!("macOS notification permission error: {:?}", unsafe { &*error });
            }
        });
        center.requestAuthorizationWithOptions_completionHandler(options, &*handler);
    }

    pub fn deliver(notif: &Notification) -> Result<(), String> {
        if !in_app_bundle() {
            return Ok(());
        }

        let center = UNUserNotificationCenter::currentNotificationCenter();

        let content = UNMutableNotificationContent::new();
        content.setTitle(&NSString::from_str(&notif.title));
        content.setBody(&NSString::from_str(&notif.body));

        // Unique id per notification so they stack rather than replace.
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let id = NSString::from_str(&format!("weechat-{}", ts));

        let request = UNNotificationRequest::requestWithIdentifier_content_trigger(
            &id,
            &*content,
            None,
        );

        center.addNotificationRequest_withCompletionHandler(&request, None);
        Ok(())
    }
}

#[cfg(target_os = "windows")]
mod backend {
    use super::Notification;
    use std::sync::OnceLock;

    pub fn init() {}

    fn icon_path() -> Option<&'static str> {
        static ICON_PATH: OnceLock<Option<String>> = OnceLock::new();
        ICON_PATH.get_or_init(|| {
            let bytes: &[u8] = include_bytes!("../../assets/icon.png");
            let path = std::env::temp_dir().join("weechat-rs-notification-icon.png");
            std::fs::write(&path, bytes).ok()?;
            Some(path.to_string_lossy().into_owned())
        }).as_deref()
    }

    pub fn deliver(notif: &Notification) -> Result<(), String> {
        let mut n = notify_rust::Notification::new();
        n.summary(&notif.title)
            .body(&notif.body)
            .appname(&notif.app_name);
        if let Some(p) = icon_path() {
            n.icon(p);
        }
        n.show().map_err(|e| format!("notify_rust: {}", e))?;
        Ok(())
    }
}
