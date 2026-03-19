use crate::cef_webview::CefBrowserExt;
use cef::*;
use objc2::ClassType;
use objc2::rc::Retained;
use objc2_app_kit::{NSResponder, NSView};
use objc2_foundation::{NSObject, NSObjectProtocol};
use objc2_foundation::{NSPoint, NSRect, NSSize};

fn restore_window_first_responder(nsview: &NSView, reason: &str) {
  let Some(window) = nsview.window() else {
    log::info!(
      "cef macos focus restore skipped reason={} browser_view={:p} state=no_window",
      reason,
      nsview
    );
    return;
  };
  let Some(first_responder) = window.firstResponder() else {
    log::info!(
      "cef macos focus restore skipped reason={} browser_view={:p} state=no_first_responder",
      reason,
      nsview
    );
    return;
  };

  let first_responder_ptr: *const NSResponder = &*first_responder;
  let first_responder_object = unsafe { first_responder_ptr.cast::<NSObject>().as_ref() };
  let Some(first_responder_object) = first_responder_object else {
    log::info!(
      "cef macos focus restore skipped reason={} browser_view={:p} state=invalid_first_responder_ptr",
      reason,
      nsview
    );
    return;
  };

  if !first_responder_object.isKindOfClass(NSView::class()) {
    log::info!(
      "cef macos focus restore skipped reason={} browser_view={:p} first_responder={:p} state=first_responder_not_view",
      reason,
      nsview,
      &*first_responder
    );
    return;
  }

  let first_responder_view = unsafe { first_responder_ptr.cast::<NSView>().as_ref() };
  let Some(first_responder_view) = first_responder_view else {
    log::info!(
      "cef macos focus restore skipped reason={} browser_view={:p} state=invalid_first_responder_view_ptr",
      reason,
      nsview
    );
    return;
  };

  if !std::ptr::eq(first_responder_view, nsview) && !first_responder_view.isDescendantOf(nsview) {
    log::info!(
      "cef macos focus restore skipped reason={} browser_view={:p} first_responder_view={:p} state=first_responder_outside_browser",
      reason,
      nsview,
      first_responder_view
    );
    return;
  }

  if let Some(content_view) = window.contentView() {
    let restored = window.makeFirstResponder(Some(&content_view));
    log::info!(
      "cef macos focus restore attempted reason={} browser_view={:p} first_responder_view={:p} content_view={:p} restored={}",
      reason,
      nsview,
      first_responder_view,
      &*content_view,
      restored
    );
  } else {
    log::warn!(
      "cef macos focus restore skipped reason={} browser_view={:p} first_responder_view={:p} state=no_content_view",
      reason,
      nsview,
      first_responder_view
    );
  }
}

impl CefBrowserExt for cef::Browser {
  fn nsview(&self) -> Option<objc2::rc::Retained<objc2_app_kit::NSView>> {
    let host = self.host()?;
    let nsview = host.window_handle() as *mut NSView;
    unsafe { Retained::<NSView>::retain(nsview) }
  }

  fn bounds(&self) -> cef::Rect {
    let Some(nsview) = self.nsview() else {
      return cef::Rect::default();
    };

    let parent = unsafe { nsview.superview().unwrap() };
    let parent_frame = parent.frame();
    let webview_frame = nsview.frame();

    cef::Rect {
      x: webview_frame.origin.x as i32,
      y: (parent_frame.size.height - webview_frame.origin.y - webview_frame.size.height) as i32,
      width: webview_frame.size.width as i32,
      height: webview_frame.size.height as i32,
    }
  }

  fn set_bounds(&self, rect: Option<&cef::Rect>) {
    let Some(rect) = rect else {
      return;
    };

    let Some(nsview) = self.nsview() else {
      return;
    };

    let parent = unsafe { nsview.superview().unwrap() };
    let parent_frame = parent.frame();

    let origin = NSPoint {
      x: rect.x as f64,
      y: (parent_frame.size.height - (rect.y as f64 + rect.height as f64)),
    };

    let size = NSSize {
      width: rect.width as f64,
      height: rect.height as f64,
    };

    nsview.setFrame(NSRect { origin, size });
  }

  fn scale_factor(&self) -> f64 {
    let Some(nsview) = self.nsview() else {
      return 1.0;
    };

    let screen = nsview.window().and_then(|w| w.screen());
    screen.map(|s| s.backingScaleFactor()).unwrap_or(1.0)
  }

  fn set_visible(&self, visible: i32) {
    let Some(nsview) = self.nsview() else {
      return;
    };

    if visible != 0 {
      log::debug!("cef macos child view visible=true browser_view={:p}", &*nsview);
      nsview.setHidden(false);
    } else {
      log::info!("cef macos child view visible=false browser_view={:p}", &*nsview);
      restore_window_first_responder(&nsview, "set_visible_false");
      nsview.setHidden(true);
    }
  }

  fn close(&self) {
    let Some(nsview) = self.nsview() else {
      return;
    };

    log::info!("cef macos child view close browser_view={:p}", &*nsview);
    restore_window_first_responder(&nsview, "close");
    nsview.removeFromSuperview();
  }

  fn set_parent(&self, parent: &cef::Window) {
    crate::cef_impl::ensure_valid_content_view(parent.window_handle());

    let Some(nsview) = self.nsview() else {
      return;
    };

    let parent_nsview = parent.window_handle();
    let Some(parent_nsview) = (unsafe { Retained::<NSView>::retain(parent_nsview as _) }) else {
      return;
    };

    parent_nsview.addSubview(&nsview);
  }
}
