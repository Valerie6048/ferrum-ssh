//! Native background effects belong behind the OpenGL view, not inside it.
use objc2_app_kit::NSView;
use raw_window_handle::{AppKitWindowHandle, HasWindowHandle, RawWindowHandle, WindowHandle};
use window_vibrancy::{LiquidGlassOptions, NSVisualEffectMaterial};

struct BackgroundHost<'a>(&'a NSView);

impl HasWindowHandle for BackgroundHost<'_> {
    fn window_handle(&self) -> Result<WindowHandle<'_>, raw_window_handle::HandleError> {
        let handle = AppKitWindowHandle::new(std::ptr::NonNull::from(self.0).cast());
        // The retained parent view lives at least as long as this borrowed handle.
        Ok(unsafe { WindowHandle::borrow_raw(RawWindowHandle::AppKit(handle)) })
    }
}

pub fn install_glass(window: &impl HasWindowHandle) -> anyhow::Result<()> {
    let RawWindowHandle::AppKit(handle) = window.window_handle()?.as_raw() else {
        return Ok(());
    };
    // eframe calls this on AppKit's main thread during window creation. The
    // handle refers to its live NSView; its parent owns the rendering view.
    let view = unsafe { handle.ns_view.cast::<NSView>().as_ref() };
    let parent = unsafe { view.superview() }
        .ok_or_else(|| anyhow::anyhow!("window has no background container"))?;
    let host = BackgroundHost(&parent);
    // Adding an effect as a child of the OpenGL view would composite it over
    // every pixel. A sibling below that view leaves text and controls sharp.
    if window_vibrancy::apply_liquid_glass(&host, LiquidGlassOptions::default()).is_err() {
        window_vibrancy::apply_vibrancy(&host, NSVisualEffectMaterial::Sidebar, None, None)?;
    }
    Ok(())
}
