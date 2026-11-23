//! iPadOS: opening, importing, saving and exporting through the system's document picker, the
//! sheet that browses iCloud Drive, On My iPad and other apps' folders.
//!
//! The shared app asks its host for a file (`Services::pick_open`, `pick_file`, `pick_save`) and
//! then reads or writes the path it gets back with ordinary file calls. `rfd`, which answers those
//! on the desktop, has no iOS backend, so this module answers them with
//! `UIDocumentPickerViewController`:
//! - Open, Import and any other "choose a file" open the picker on the file itself, not a copy
//!   (`asCopy: false`): the user's design stays where it is, and Save writes back into it. iPadOS
//!   gives leave to change that one file, not to create files beside it, so the atomic save's
//!   temporary file is refused and `solidcraft_io::safe_write` writes into the file itself there.
//! - Save As and Export first make an empty file of the chosen name in the app's temporary folder
//!   and let the picker move it where the user says (`initForExportingURLs`, `asCopy: false`); the
//!   answer is the moved file, which the app may then write, exactly as a file opened above.
//! - Every file picked is kept as a bookmark ([`crate::bookmarks`]): the leave to use it ends when
//!   the app quits, so a recent design comes back through `Services::restore_file`, which resolves
//!   its bookmark and hands the path to the shell to open ([`take`]).
//!
//! Each call to UIKit and Foundation here is on the main thread (the UI's), where eframe calls the
//! services and UIKit calls the delegate. Starting access to a security-scoped file, reading a
//! bookmark and defining the delegate's class have no safe API in objc2, hence the `unsafe_code`
//! allowances on [`answer`], [`restore`] and the class below, each with its reason.

use std::cell::RefCell;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::rc::Rc;
use std::sync::{Mutex, OnceLock, PoisonError};
use std::task::{Context, Poll, Waker};

use objc2::rc::Retained;
use objc2::runtime::{Bool, NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_foundation::{NSArray, NSData, NSString, NSURL, NSURLBookmarkCreationOptions, NSURLBookmarkResolutionOptions};
use objc2_ui_kit::{UIApplication, UIDocumentPickerDelegate, UIDocumentPickerViewController, UIModalPresentationStyle, UIViewController};
use objc2_uniform_type_identifiers::UTType;
use solidcraft_engine::io;
use solidcraft_ui_egui::{FilePickerFuture, Services};

use crate::bookmarks::Store;

/// Paths of recent designs brought back, waiting for the shell to open them.
static PENDING: Mutex<Vec<String>> = Mutex::new(Vec::new());
/// The UI to wake when one arrives.
static UI: OnceLock<egui::Context> = OnceLock::new();

thread_local! {
    /// The bookmarks, read once from the app's data folder.
    static BOOKMARKS: RefCell<Option<Store>> = const { RefCell::new(None) };
    /// The last picker's delegate: UIKit holds a delegate weakly, so this keeps it alive while its
    /// picker is up (and until the next picker replaces it).
    static LIVE: RefCell<Option<Retained<Delegate>>> = const { RefCell::new(None) };
}

/// Fill in the services that choose files. Call on the main thread once the window exists.
pub fn install(services: &mut Services, ctx: &egui::Context) {
    // Set once; a second call would hand over the same context.
    let _ = UI.set(ctx.clone());
    services.pick_open = Some(Box::new(|first: &[&str]| pick(Ask::Open(open_extensions(first)))));
    services.pick_file = Some(Box::new(|_label: &str, exts: &[&str]| pick(Ask::Open(exts.iter().map(|e| e.to_string()).collect()))));
    services.pick_save = Some(Box::new(|name: &str, _exts: &[&str]| pick(Ask::Save(name.to_string()))));
    services.can_restore = Some(Box::new(|path: &str| with_bookmarks(|b| b.contains(path))));
    services.restore_file = Some(Box::new(restore));
}

/// The recent designs brought back since the last call, to open.
pub fn take() -> Vec<String> {
    std::mem::take(&mut *PENDING.lock().unwrap_or_else(PoisonError::into_inner))
}

/// Files the system asked the app to open (a tap on a design in Files, "Open in SolidCraft"),
/// to open. Each is kept as a bookmark, as a picked file is, so it stays in the recent list.
pub fn take_opened() -> Vec<String> {
    let mut paths = Vec::new();
    for path in winit::platform::ios::take_opened_files() {
        let Some(text) = path.to_str() else { continue };
        // The window layer started the file's security scope, so a URL of its path reaches it.
        let url = NSURL::fileURLWithPath(&NSString::from_str(text));
        remember(text, &url);
        paths.push(text.to_string());
    }
    paths
}

/// What the picker is for.
enum Ask {
    /// A file to read, of these extensions (none: anything).
    Open(Vec<String>),
    /// A place and a name for a file to write, this name offered.
    Save(String),
}

/// The open picker's types: the kind the command reads first, then everything the app opens (as
/// the desktop's dialog offers, from the format registry).
fn open_extensions(first: &[&str]) -> Vec<String> {
    let mut all: Vec<String> = first.iter().map(|e| e.to_string()).collect();
    if first.is_empty() {
        let mut everything = io::formats::open_extensions(true);
        everything.extend(["dxf", "svg"]);
        for (_, exts) in io::formats::import_filters(true) {
            everything.extend(exts);
        }
        for e in everything {
            if !all.iter().any(|a| a.eq_ignore_ascii_case(e)) {
                all.push(e.to_string());
            }
        }
    }
    all
}

/// One picker's answer, shared by the delegate (which sets it) and the future (which waits).
#[derive(Default)]
struct Slot {
    answer: Option<Option<String>>,
    waker: Option<Waker>,
}

type Shared = Rc<RefCell<Slot>>;

struct Answer(Shared);

impl Future for Answer {
    type Output = Option<String>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<String>> {
        let mut slot = self.0.borrow_mut();
        match slot.answer.take() {
            Some(answer) => Poll::Ready(answer),
            None => {
                slot.waker = Some(cx.waker().clone());
                Poll::Pending
            }
        }
    }
}

fn settle(shared: &Shared, answer: Option<String>) {
    let mut slot = shared.borrow_mut();
    slot.answer = Some(answer);
    if let Some(waker) = slot.waker.take() {
        waker.wake();
    }
}

/// Show the picker for `ask`; the future answers with the chosen file's path, or `None` when the
/// user cancels or the picker can't be shown.
fn pick(ask: Ask) -> FilePickerFuture {
    let shared: Shared = Rc::default();
    if let Err(why) = present(&ask, &shared) {
        log::warn!(target: "solidcraft_ios", "files: the picker could not open: {why}");
        settle(&shared, None);
    }
    Box::pin(Answer(shared))
}

fn present(ask: &Ask, shared: &Shared) -> Result<(), String> {
    let mtm = MainThreadMarker::new().ok_or("not on the main thread")?;
    let top = top_controller(mtm).ok_or("no window to show it over")?;
    let (picker, placeholder) = match ask {
        Ask::Open(exts) => {
            let types = content_types(exts);
            let picker = UIDocumentPickerViewController::initForOpeningContentTypes_asCopy(UIDocumentPickerViewController::alloc(mtm), &types, false);
            (picker, None)
        }
        Ask::Save(name) => {
            let file = placeholder(name).map_err(|e| format!("the file to move could not be made: {e}"))?;
            let url = NSURL::fileURLWithPath(&NSString::from_str(&file.to_string_lossy()));
            let picker = UIDocumentPickerViewController::initForExportingURLs_asCopy(
                UIDocumentPickerViewController::alloc(mtm),
                &NSArray::from_retained_slice(&[url]),
                false,
            );
            (picker, Some(file))
        }
    };
    picker.setAllowsMultipleSelection(false);
    picker.setShouldShowFileExtensions(true);
    picker.setModalPresentationStyle(UIModalPresentationStyle::FormSheet);
    let delegate = Delegate::new(mtm, shared.clone(), placeholder);
    picker.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
    LIVE.with(|live| *live.borrow_mut() = Some(delegate));
    top.presentViewController_animated_completion(&picker, true, None);
    Ok(())
}

/// The types the picker lets the user choose: each extension's type (the system's, or a dynamic
/// one for an extension it doesn't know), or any data when there are none.
fn content_types(exts: &[String]) -> Retained<NSArray<UTType>> {
    let mut types: Vec<Retained<UTType>> = exts.iter().filter_map(|e| UTType::typeWithFilenameExtension(&NSString::from_str(e))).collect();
    if types.is_empty() {
        types.extend(UTType::typeWithIdentifier(&NSString::from_str("public.data")));
    }
    NSArray::from_retained_slice(&types)
}

/// The view controller on top, which the picker is shown over. The iPad app has one window and
/// no scenes (winit 0.30), so the application's key window is the app's.
#[allow(deprecated)]
fn top_controller(mtm: MainThreadMarker) -> Option<Retained<UIViewController>> {
    let app = UIApplication::sharedApplication(mtm);
    let window = app.keyWindow().or_else(|| app.windows().firstObject())?;
    let mut top = window.rootViewController()?;
    while let Some(next) = top.presentedViewController() {
        top = next;
    }
    Some(top)
}

/// An empty file named `name` in a folder of its own in the app's temporary folder, for the export
/// picker to move.
fn placeholder(name: &str) -> std::io::Result<PathBuf> {
    let name = Path::new(name).file_name().map(|n| n.to_string_lossy().into_owned()).filter(|n| !n.is_empty()).unwrap_or_else(|| "Untitled".into());
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let dir = std::env::temp_dir().join("save-as").join(nanos.to_string());
    std::fs::create_dir_all(&dir)?;
    let file = dir.join(name);
    std::fs::write(&file, b"")?;
    Ok(file)
}

/// The picker chose `url`: start using it, keep its bookmark, and answer with its path.
#[allow(unsafe_code)]
fn answer(url: &NSURL) -> Option<String> {
    // SAFETY: a URL the document picker handed over, on the main thread. Access is never stopped:
    // the app may save into the file for as long as it runs, and iPadOS ends it at quit.
    let started = unsafe { url.startAccessingSecurityScopedResource() };
    if !started {
        // A file in the app's own container needs no leave and says so.
        log::info!(target: "solidcraft_ios", "files: no security scope to start (a file of the app's own)");
    }
    let path = url.path()?.to_string();
    remember(&path, url);
    Some(path)
}

/// Keep `url`'s bookmark under `path`, and write the store.
fn remember(path: &str, url: &NSURL) {
    match url.bookmarkDataWithOptions_includingResourceValuesForKeys_relativeToURL_error(NSURLBookmarkCreationOptions(0), None, None) {
        Ok(data) => {
            let bytes = data.to_vec();
            with_bookmarks(|b| {
                b.put(path, &bytes);
                write_bookmarks(b);
            });
        }
        Err(e) => log::warn!(target: "solidcraft_ios", "files: no bookmark for a picked file: {}", e.localizedDescription()),
    }
}

/// A recent design that isn't reachable now: resolve its bookmark, start using the file, and queue
/// its path (the path it has now, which may differ) for the shell to open. Without a bookmark, or
/// when it no longer resolves, the path is queued as it was, so opening it says what is wrong.
#[allow(unsafe_code)]
fn restore(path: &str) {
    let opened = with_bookmarks(|b| b.get(path)).and_then(|bytes| {
        let data = NSData::with_bytes(&bytes);
        let mut stale = Bool::NO;
        // SAFETY: `stale` is a valid pointer for the call; the data is a bookmark this app made.
        let url = unsafe {
            NSURL::URLByResolvingBookmarkData_options_relativeToURL_bookmarkDataIsStale_error(
                &data,
                NSURLBookmarkResolutionOptions(0),
                None,
                &mut stale,
            )
        };
        match url {
            Ok(url) => {
                // SAFETY: as in `answer`: a URL from this app's own bookmark, on the main thread.
                let _ = unsafe { url.startAccessingSecurityScopedResource() };
                let now = url.path()?.to_string();
                if stale.as_bool() || now != path {
                    remember(&now, &url);
                }
                Some(now)
            }
            Err(e) => {
                log::warn!(target: "solidcraft_ios", "files: a recent design's bookmark did not resolve: {}", e.localizedDescription());
                None
            }
        }
    });
    PENDING.lock().unwrap_or_else(PoisonError::into_inner).push(opened.unwrap_or_else(|| path.to_string()));
    if let Some(ctx) = UI.get() {
        ctx.request_repaint();
    }
}

fn bookmarks_path() -> Option<PathBuf> {
    solidcraft_engine::recovery::data_dir().map(|d| d.join("picked-files.json"))
}

fn with_bookmarks<T>(f: impl FnOnce(&mut Store) -> T) -> T {
    BOOKMARKS.with(|cell| {
        let mut cell = cell.borrow_mut();
        let store =
            cell.get_or_insert_with(|| bookmarks_path().and_then(|p| std::fs::read_to_string(p).ok()).map(|t| Store::parse(&t)).unwrap_or_default());
        f(store)
    })
}

fn write_bookmarks(store: &Store) {
    let Some(path) = bookmarks_path() else { return };
    let written =
        path.parent().map_or(Ok(()), std::fs::create_dir_all).and_then(|()| io::safe_write::write_atomic(&path, store.to_json().as_bytes(), false));
    if let Err(e) = written {
        log::warn!(target: "solidcraft_ios", "files: the picked files' bookmarks were not written: {e}");
    }
}

/// What the delegate holds: where its answer goes, and the empty file a Save As made.
struct Ivars {
    shared: Shared,
    placeholder: Option<PathBuf>,
}

// The class definition is `unsafe impl`s by objc2's design; their reasons are inside.
#[allow(unsafe_code)]
mod class {
    use super::*;

    define_class!(
        // SAFETY:
        // - NSObject has no subclassing requirements.
        // - `Delegate` doesn't implement `Drop`.
        #[unsafe(super(NSObject))]
        #[thread_kind = MainThreadOnly]
        #[name = "SolidCraftDocumentPickerDelegate"]
        #[ivars = Ivars]
        pub(super) struct Delegate;

        // SAFETY: NSObjectProtocol has no safety requirements.
        unsafe impl NSObjectProtocol for Delegate {}

        // SAFETY: the methods' signatures are the protocol's (objc2-ui-kit's declarations).
        unsafe impl UIDocumentPickerDelegate for Delegate {
            #[unsafe(method(documentPicker:didPickDocumentsAtURLs:))]
            fn did_pick(&self, _controller: &UIDocumentPickerViewController, urls: &NSArray<NSURL>) {
                let picked = urls.firstObject().and_then(|url| answer(&url));
                self.finish(picked);
            }

            #[unsafe(method(documentPickerWasCancelled:))]
            fn cancelled(&self, _controller: &UIDocumentPickerViewController) {
                if let Some(file) = &self.ivars().placeholder {
                    // Not moved anywhere: the empty file and its folder go.
                    let _ = std::fs::remove_file(file);
                    if let Some(dir) = file.parent() {
                        let _ = std::fs::remove_dir(dir);
                    }
                }
                self.finish(None);
            }
        }
    );
}

use class::Delegate;

impl Delegate {
    #[allow(unsafe_code)]
    fn new(mtm: MainThreadMarker, shared: Shared, placeholder: Option<PathBuf>) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(Ivars { shared, placeholder });
        // SAFETY: NSObject's `init` on a freshly allocated object.
        unsafe { msg_send![super(this), init] }
    }

    /// Answer. The delegate itself stays in [`LIVE`] until the next picker replaces it: dropping
    /// it here, inside its own method, could free it while UIKit's call is still running.
    fn finish(&self, picked: Option<String>) {
        settle(&self.ivars().shared, picked);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_open_picker_offers_the_commands_kind_first_then_everything_the_app_opens() {
        let all = open_extensions(&[]);
        assert!(all.iter().any(|e| e == io::DESIGN_EXT));
        assert!(all.iter().any(|e| e.eq_ignore_ascii_case("step")));
        assert!(all.iter().any(|e| e == "dxf") && all.iter().any(|e| e == "svg"));
        assert_eq!(open_extensions(&["png", "jpg"]), vec!["png".to_string(), "jpg".to_string()]);
    }
}
