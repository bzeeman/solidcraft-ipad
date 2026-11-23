//! SolidCraft: whole file. The UIScene life cycle.
//!
//! An app linked against the iOS 27 SDK starts only if it has a scene life cycle, and a visionOS
//! port needs one too. Upstream winit 0.30 (and 0.31) has none: it starts `UIApplicationMain` with
//! no delegate and makes its windows with `initWithFrame:`, which iOS 13 to 26 still accept.
//!
//! The app names [`WinitSceneDelegate`] in its Info.plist (`UIApplicationSceneManifest`, one
//! scene, no storyboard). Then:
//! - `scene:willConnectToSession:options:` keeps the window scene; windows made after it are made
//!   in that scene (`initWithWindowScene:`), and a window made before it is moved into it.
//! - The scene's activation and foreground changes give winit's `Resumed`, `Suspended` and
//!   `Occluded`. The application's notifications give them too (apps without the manifest, and
//!   whichever arrives first): [`Lifecycle`] sends each change once.
//! - Files the system hands the app (a tap on a design in Files, "Open in SolidCraft") arrive in
//!   the connection options or `scene:openURLContexts:`. Their security scope is started here and
//!   their paths wait in [`take_opened_files`] (`platform::ios::take_opened_files`).
//!
//! Without the manifest nothing here runs and winit behaves as upstream.

use std::cell::{Cell, RefCell};
use std::path::PathBuf;

use objc2::rc::Retained;
use objc2::{declare_class, mutability, ClassType, DeclaredClass};
use objc2_foundation::{MainThreadMarker, NSObject, NSObjectProtocol, NSSet};
use objc2_ui_kit::{
    UIApplication, UIOpenURLContext, UIScene, UISceneConnectionOptions, UISceneDelegate,
    UISceneSession, UIWindowScene, UIWindowSceneDelegate,
};

use super::app_state::{self, EventWrapper};
use super::window::WinitUIWindow;
use crate::event::Event;

thread_local! {
    /// The connected window scene, once there is one.
    static SCENE: RefCell<Option<Retained<UIWindowScene>>> = const { RefCell::new(None) };
    /// Windows made before the scene connected, to move into it when it does.
    static WAITING: RefCell<Vec<Retained<WinitUIWindow>>> = const { RefCell::new(Vec::new()) };
    /// Paths of files the system asked the app to open, oldest first.
    static OPENED: RefCell<Vec<PathBuf>> = const { RefCell::new(Vec::new()) };
    static LIFECYCLE: Cell<Lifecycle> = const { Cell::new(Lifecycle::LAUNCHED) };
}

/// Whether the app is active and in the foreground, as last told to the event handler.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Lifecycle {
    active: bool,
    foreground: bool,
}

/// What to tell the event handler about a change.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Change {
    Resumed,
    Suspended,
    /// `Occluded(true)` for every window.
    Hidden,
    /// `Occluded(false)` for every window.
    Shown,
}

impl Lifecycle {
    /// Launched: inactive and, as upstream winit assumes, visible (no `Occluded(false)` first).
    pub(crate) const LAUNCHED: Self = Self { active: false, foreground: true };

    /// The state after the system said the app became active (`true`) or will resign (`false`),
    /// and what to send: nothing when it repeats what was already sent.
    pub(crate) fn activate(self, active: bool) -> (Self, Option<Change>) {
        if self.active == active {
            return (self, None);
        }
        let change = if active { Change::Resumed } else { Change::Suspended };
        (Self { active, ..self }, Some(change))
    }

    /// The state after the app entered the foreground (`true`) or background (`false`).
    pub(crate) fn foreground(self, foreground: bool) -> (Self, Option<Change>) {
        if self.foreground == foreground {
            return (self, None);
        }
        let change = if foreground { Change::Shown } else { Change::Hidden };
        (Self { foreground, ..self }, Some(change))
    }
}

fn apply(mtm: MainThreadMarker, step: impl FnOnce(Lifecycle) -> (Lifecycle, Option<Change>)) {
    let (next, change) = step(LIFECYCLE.get());
    LIFECYCLE.set(next);
    let event = |e| app_state::handle_nonuser_event(mtm, EventWrapper::StaticEvent(e));
    match change {
        None => {},
        Some(Change::Resumed) => event(Event::Resumed),
        Some(Change::Suspended) => event(Event::Suspended),
        Some(Change::Hidden) => {
            app_state::send_occluded_event_for_all_windows(&UIApplication::sharedApplication(mtm), true)
        },
        Some(Change::Shown) => app_state::send_occluded_event_for_all_windows(
            &UIApplication::sharedApplication(mtm),
            false,
        ),
    }
}

/// The app became active (`true`) or will resign active (`false`).
pub(crate) fn set_active(mtm: MainThreadMarker, active: bool) {
    apply(mtm, |l| l.activate(active))
}

/// The app entered the foreground (`true`) or the background (`false`).
pub(crate) fn set_foreground(mtm: MainThreadMarker, foreground: bool) {
    apply(mtm, |l| l.foreground(foreground))
}

/// The connected window scene, if the app has a scene life cycle and it connected.
pub(crate) fn current(_mtm: MainThreadMarker) -> Option<Retained<UIWindowScene>> {
    SCENE.with_borrow(|s| s.clone())
}

/// `window` was made before the scene connected: move it into the scene when it does.
pub(crate) fn wait_for_scene(_mtm: MainThreadMarker, window: &Retained<WinitUIWindow>) {
    WAITING.with_borrow_mut(|w| w.push(window.clone()));
}

/// The paths of the files the system asked the app to open since the last call, oldest first.
pub(crate) fn take_opened_files(_mtm: MainThreadMarker) -> Vec<PathBuf> {
    OPENED.with_borrow_mut(std::mem::take)
}

/// Keep the files of `contexts` for [`take_opened_files`]. Their security scope is started and
/// never stopped: the app reads and writes them by path for as long as it runs.
fn receive(contexts: &NSSet<UIOpenURLContext>) -> bool {
    let mut any = false;
    for context in contexts.iter() {
        let url = unsafe { context.URL() };
        if !unsafe { url.isFileURL() } {
            continue;
        }
        // False for a file already inside the app's container (a copy in Documents/Inbox); it
        // is readable anyway.
        let _ = unsafe { url.startAccessingSecurityScopedResource() };
        if let Some(path) = unsafe { url.path() } {
            OPENED.with_borrow_mut(|o| o.push(PathBuf::from(path.to_string())));
            any = true;
        }
    }
    any
}

declare_class!(
    /// The scene delegate the app's Info.plist names (`UISceneDelegateClassName`).
    pub(crate) struct WinitSceneDelegate;

    unsafe impl ClassType for WinitSceneDelegate {
        type Super = NSObject;
        type Mutability = mutability::MainThreadOnly;
        const NAME: &'static str = "WinitSceneDelegate";
    }

    impl DeclaredClass for WinitSceneDelegate {}

    unsafe impl NSObjectProtocol for WinitSceneDelegate {}

    unsafe impl UISceneDelegate for WinitSceneDelegate {
        #[method(scene:willConnectToSession:options:)]
        fn scene_will_connect(
            &self,
            scene: &UIScene,
            _session: &UISceneSession,
            options: &UISceneConnectionOptions,
        ) {
            let ptr: *const UIScene = scene;
            // SAFETY: a window scene is checked for before the cast.
            let scene = scene
                .is_kind_of::<UIWindowScene>()
                .then(|| unsafe { Retained::retain(ptr.cast_mut().cast::<UIWindowScene>()) })
                .flatten();
            if let Some(scene) = scene {
                SCENE.with_borrow_mut(|s| *s = Some(scene.clone()));
                for window in WAITING.with_borrow_mut(std::mem::take) {
                    unsafe { window.setWindowScene(Some(&scene)) };
                    window.makeKeyAndVisible();
                }
            }
            // Started by a tap on a file: the app's first frame takes it.
            let _ = receive(&*unsafe { options.URLContexts() });
        }

        #[method(sceneDidDisconnect:)]
        fn scene_did_disconnect(&self, scene: &UIScene) {
            SCENE.with_borrow_mut(|s| {
                if s.as_deref().is_some_and(|mine| std::ptr::eq::<UIScene>(&**mine, scene)) {
                    *s = None;
                }
            });
        }

        #[method(sceneDidBecomeActive:)]
        fn scene_did_become_active(&self, _scene: &UIScene) {
            set_active(MainThreadMarker::from(self), true);
        }

        #[method(sceneWillResignActive:)]
        fn scene_will_resign_active(&self, _scene: &UIScene) {
            set_active(MainThreadMarker::from(self), false);
        }

        #[method(sceneWillEnterForeground:)]
        fn scene_will_enter_foreground(&self, _scene: &UIScene) {
            set_foreground(MainThreadMarker::from(self), true);
        }

        #[method(sceneDidEnterBackground:)]
        fn scene_did_enter_background(&self, _scene: &UIScene) {
            set_foreground(MainThreadMarker::from(self), false);
        }

        #[method(scene:openURLContexts:)]
        fn scene_open_url_contexts(&self, _scene: &UIScene, contexts: &NSSet<UIOpenURLContext>) {
            if receive(contexts) {
                // The app may be idle: draw a frame so that it looks at the new files.
                app_state::request_redraw_all_windows(MainThreadMarker::from(self));
            }
        }
    }

    unsafe impl UIWindowSceneDelegate for WinitSceneDelegate {}
);

impl WinitSceneDelegate {
    /// Register the class, so that UIKit finds it by the name in the Info.plist. Call before
    /// `UIApplicationMain`.
    pub(crate) fn register() {
        let _ = Self::class();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_change_is_sent_once_whoever_reports_it() {
        let l = Lifecycle::LAUNCHED;
        // The scene and the application both report the activation.
        let (l, c) = l.activate(true);
        assert_eq!(c, Some(Change::Resumed));
        let (l, c) = l.activate(true);
        assert_eq!(c, None);
        // Launched visible: entering the foreground at launch sends nothing.
        let (l, c) = l.foreground(true);
        assert_eq!(c, None);
        let (l, c) = l.activate(false);
        assert_eq!(c, Some(Change::Suspended));
        let (l, c) = l.foreground(false);
        assert_eq!(c, Some(Change::Hidden));
        let (l, c) = l.foreground(false);
        assert_eq!(c, None);
        let (l, c) = l.foreground(true);
        assert_eq!(c, Some(Change::Shown));
        let (_, c) = l.activate(true);
        assert_eq!(c, Some(Change::Resumed));
    }
}
