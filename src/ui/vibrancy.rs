//! macOS background blur: an `NSVisualEffectView` placed *behind* winit's
//! content view (as a sibling in the window frame view), so the translucent
//! egui surface is composited over a live blur of the desktop.
//!
//! Adding the effect view as a subview of the content view (what most crates
//! do) would cover the OpenGL surface instead of sitting behind it.

use std::cell::RefCell;

use objc2::rc::Retained;
use objc2::MainThreadMarker;
use objc2_app_kit::{
    NSAutoresizingMaskOptions, NSView, NSVisualEffectBlendingMode, NSVisualEffectMaterial,
    NSVisualEffectState, NSVisualEffectView, NSWindowOrderingMode,
};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

thread_local! {
    static EFFECT_VIEW: RefCell<Option<Retained<NSVisualEffectView>>> = const { RefCell::new(None) };
}

pub fn set_blur(window: &impl HasWindowHandle, enabled: bool) -> Result<(), String> {
    let mtm = MainThreadMarker::new().ok_or("not on the main thread")?;

    EFFECT_VIEW.with_borrow_mut(|slot| {
        if let Some(old) = slot.take() {
            old.removeFromSuperview();
        }
        if !enabled {
            return Ok(());
        }

        let handle = window.window_handle().map_err(|e| e.to_string())?;
        let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
            return Err("not an AppKit window".into());
        };
        // SAFETY: winit hands out a valid NSView for the lifetime of the window.
        let content: &NSView = unsafe { handle.ns_view.cast().as_ref() };
        let frame_view = unsafe { content.superview() }.ok_or("content view has no superview")?;

        let fx = NSVisualEffectView::initWithFrame(mtm.alloc(), frame_view.bounds());
        fx.setMaterial(NSVisualEffectMaterial::UnderWindowBackground);
        fx.setBlendingMode(NSVisualEffectBlendingMode::BehindWindow);
        fx.setState(NSVisualEffectState::Active);
        fx.setAutoresizingMask(
            NSAutoresizingMaskOptions::ViewWidthSizable | NSAutoresizingMaskOptions::ViewHeightSizable,
        );
        frame_view.addSubview_positioned_relativeTo(&fx, NSWindowOrderingMode::Below, Some(content));
        *slot = Some(fx);
        Ok(())
    })
}
