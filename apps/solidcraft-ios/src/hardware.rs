//! Whether a physical keyboard is connected to the iPad (a Magic Keyboard, a Bluetooth keyboard):
//! the Game Controller framework's `GCKeyboard` has a keyboard to offer exactly then.
//!
//! objc2 has no binding of that framework among this crate's dependencies, so the class is looked
//! up by name and asked with `msg_send!`; this is the one other place besides [`crate::documents`]
//! that allows `unsafe`, each use with its reason.

use std::ffi::c_void;

use objc2::msg_send;
use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject};

// SAFETY: declares one global of the framework, read below without being dereferenced.
#[allow(unsafe_code)]
#[link(name = "GameController", kind = "framework")]
unsafe extern "C" {
    /// The name of the notice a keyboard's connection sends. Only read so that the linker keeps
    /// the framework, whose classes are otherwise looked up only by name.
    static GCKeyboardDidConnectNotification: *const c_void;
}

/// Whether a physical keyboard is connected now. False when the framework or its class is missing.
pub fn keyboard_connected() -> bool {
    // SAFETY: reading the address of an immutable global the framework defines; nothing is
    // dereferenced.
    #[allow(unsafe_code)]
    std::hint::black_box(unsafe { GCKeyboardDidConnectNotification });
    let Some(class) = AnyClass::get(c"GCKeyboard") else { return false };
    // SAFETY: `+[GCKeyboard coalescedKeyboard]` takes no arguments and returns a GCKeyboard or nil
    // (iOS 14 and later; the app needs 16). objc2 retains the autoreleased result.
    #[allow(unsafe_code)]
    let keyboard: Option<Retained<AnyObject>> = unsafe { msg_send![class, coalescedKeyboard] };
    keyboard.is_some()
}
