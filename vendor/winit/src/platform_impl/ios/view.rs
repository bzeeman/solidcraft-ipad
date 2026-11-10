#![allow(clippy::unnecessary_cast)]
use std::cell::{Cell, RefCell};

use objc2::rc::Retained;
use objc2::runtime::{AnyClass, NSObjectProtocol, ProtocolObject};
use objc2::{declare_class, msg_send, msg_send_id, mutability, sel, ClassType, DeclaredClass};
use objc2_foundation::{CGFloat, CGPoint, CGRect, MainThreadMarker, NSObject, NSSet, NSString};
use objc2_ui_kit::{
    UICoordinateSpace, UIEvent, UIForceTouchCapability, UIGestureRecognizer,
    UIGestureRecognizerDelegate, UIGestureRecognizerState, UIHoverGestureRecognizer, UIKeyInput,
    UIPanGestureRecognizer, UIPencilInteraction, UIPencilInteractionDelegate,
    UIPencilInteractionPhase, UIPencilInteractionSqueeze, UIPencilInteractionTap,
    UIPinchGestureRecognizer, UIResponder, UIRotationGestureRecognizer, UITapGestureRecognizer,
    UITextInputTraits, UITouch, UITouchPhase, UITouchType, UITraitEnvironment, UIView,
};

use super::app_state::{self, EventWrapper};
use super::window::WinitUIWindow;
use crate::dpi::PhysicalPosition;
use crate::event::{ElementState, Event, Force, KeyEvent, Touch, TouchPhase, WindowEvent};
use crate::keyboard::{Key, KeyCode, KeyLocation, NamedKey, NativeKeyCode, PhysicalKey};
use crate::platform_impl::platform::DEVICE_ID;
use crate::platform_impl::KeyEventExtra;
use crate::window::{WindowAttributes, WindowId as RootWindowId};

pub struct WinitViewState {
    pinch_gesture_recognizer: RefCell<Option<Retained<UIPinchGestureRecognizer>>>,
    doubletap_gesture_recognizer: RefCell<Option<Retained<UITapGestureRecognizer>>>,
    rotation_gesture_recognizer: RefCell<Option<Retained<UIRotationGestureRecognizer>>>,
    pan_gesture_recognizer: RefCell<Option<Retained<UIPanGestureRecognizer>>>,

    // for iOS delta references the start of the Gesture
    rotation_last_delta: Cell<CGFloat>,
    pinch_last_delta: Cell<CGFloat>,
    pan_last_delta: Cell<CGPoint>,

    // SolidCraft: the touches in contact with the screen. A hover ends when the Pencil's tip
    // lands, and that is not the pointer leaving.
    touches_down: Cell<usize>,
}

declare_class!(
    pub(crate) struct WinitView;

    unsafe impl ClassType for WinitView {
        #[inherits(UIResponder, NSObject)]
        type Super = UIView;
        type Mutability = mutability::MainThreadOnly;
        const NAME: &'static str = "WinitUIView";
    }

    impl DeclaredClass for WinitView {
        type Ivars = WinitViewState;
    }

    unsafe impl WinitView {
        #[method(drawRect:)]
        fn draw_rect(&self, rect: CGRect) {
            let mtm = MainThreadMarker::new().unwrap();
            let window = self.window().unwrap();
            app_state::handle_nonuser_event(
                mtm,
                EventWrapper::StaticEvent(Event::WindowEvent {
                    window_id: RootWindowId(window.id()),
                    event: WindowEvent::RedrawRequested,
                }),
            );
            let _: () = unsafe { msg_send![super(self), drawRect: rect] };
        }

        #[method(layoutSubviews)]
        fn layout_subviews(&self) {
            let mtm = MainThreadMarker::new().unwrap();
            let _: () = unsafe { msg_send![super(self), layoutSubviews] };

            let window = self.window().unwrap();
            let window_bounds = window.bounds();
            let screen = window.screen();
            let screen_space = screen.coordinateSpace();
            let screen_frame = self.convertRect_toCoordinateSpace(window_bounds, &screen_space);
            let scale_factor = screen.scale();
            let size = crate::dpi::LogicalSize {
                width: screen_frame.size.width as f64,
                height: screen_frame.size.height as f64,
            }
            .to_physical(scale_factor as f64);

            // If the app is started in landscape, the view frame and window bounds can be mismatched.
            // The view frame will be in portrait and the window bounds in landscape. So apply the
            // window bounds to the view frame to make it consistent.
            let view_frame = self.frame();
            if view_frame != window_bounds {
                self.setFrame(window_bounds);
            }

            app_state::handle_nonuser_event(
                mtm,
                EventWrapper::StaticEvent(Event::WindowEvent {
                    window_id: RootWindowId(window.id()),
                    event: WindowEvent::Resized(size),
                }),
            );
        }

        #[method(setContentScaleFactor:)]
        fn set_content_scale_factor(&self, untrusted_scale_factor: CGFloat) {
            let mtm = MainThreadMarker::new().unwrap();
            let _: () =
                unsafe { msg_send![super(self), setContentScaleFactor: untrusted_scale_factor] };

            // `window` is null when `setContentScaleFactor` is invoked prior to `[UIWindow
            // makeKeyAndVisible]` at window creation time (either manually or internally by
            // UIKit when the `UIView` is first created), in which case we send no events here
            let window = match self.window() {
                Some(window) => window,
                None => return,
            };
            // `setContentScaleFactor` may be called with a value of 0, which means "reset the
            // content scale factor to a device-specific default value", so we can't use the
            // parameter here. We can query the actual factor using the getter
            let scale_factor = self.contentScaleFactor();
            assert!(
                !scale_factor.is_nan()
                    && scale_factor.is_finite()
                    && scale_factor.is_sign_positive()
                    && scale_factor > 0.0,
                "invalid scale_factor set on UIView",
            );
            let scale_factor = scale_factor as f64;
            let bounds = self.bounds();
            let screen = window.screen();
            let screen_space = screen.coordinateSpace();
            let screen_frame = self.convertRect_toCoordinateSpace(bounds, &screen_space);
            let size = crate::dpi::LogicalSize {
                width: screen_frame.size.width as f64,
                height: screen_frame.size.height as f64,
            };
            let window_id = RootWindowId(window.id());
            app_state::handle_nonuser_events(
                mtm,
                std::iter::once(EventWrapper::ScaleFactorChanged(
                    app_state::ScaleFactorChanged {
                        window,
                        scale_factor,
                        suggested_size: size.to_physical(scale_factor),
                    },
                ))
                .chain(std::iter::once(EventWrapper::StaticEvent(
                    Event::WindowEvent {
                        window_id,
                        event: WindowEvent::Resized(size.to_physical(scale_factor)),
                    },
                ))),
            );
        }

        #[method(touchesBegan:withEvent:)]
        fn touches_began(&self, touches: &NSSet<UITouch>, _event: Option<&UIEvent>) {
            self.handle_touches(touches)
        }

        #[method(touchesMoved:withEvent:)]
        fn touches_moved(&self, touches: &NSSet<UITouch>, _event: Option<&UIEvent>) {
            self.handle_touches(touches)
        }

        #[method(touchesEnded:withEvent:)]
        fn touches_ended(&self, touches: &NSSet<UITouch>, _event: Option<&UIEvent>) {
            self.handle_touches(touches)
        }

        #[method(touchesCancelled:withEvent:)]
        fn touches_cancelled(&self, touches: &NSSet<UITouch>, _event: Option<&UIEvent>) {
            self.handle_touches(touches)
        }

        #[method(pinchGesture:)]
        fn pinch_gesture(&self, recognizer: &UIPinchGestureRecognizer) {
            let window = self.window().unwrap();

            let (phase, delta) = match recognizer.state() {
                UIGestureRecognizerState::Began => {
                    self.ivars().pinch_last_delta.set(recognizer.scale());
                    (TouchPhase::Started, 0.0)
                }
                UIGestureRecognizerState::Changed => {
                    let last_scale: f64 = self.ivars().pinch_last_delta.replace(recognizer.scale());
                    (TouchPhase::Moved, recognizer.scale() - last_scale)
                }
                UIGestureRecognizerState::Ended => {
                    let last_scale: f64 = self.ivars().pinch_last_delta.replace(0.0);
                    (TouchPhase::Moved, recognizer.scale() - last_scale)
                }
                UIGestureRecognizerState::Cancelled | UIGestureRecognizerState::Failed => {
                    self.ivars().rotation_last_delta.set(0.0);
                    // Pass -delta so that action is reversed
                    (TouchPhase::Cancelled, -recognizer.scale())
                }
                state => panic!("unexpected recognizer state: {state:?}"),
            };

            let gesture_event = EventWrapper::StaticEvent(Event::WindowEvent {
                window_id: RootWindowId(window.id()),
                event: WindowEvent::PinchGesture {
                    device_id: DEVICE_ID,
                    delta: delta as f64,
                    phase,
                },
            });

            let mtm = MainThreadMarker::new().unwrap();
            app_state::handle_nonuser_event(mtm, gesture_event);
        }

        #[method(doubleTapGesture:)]
        fn double_tap_gesture(&self, recognizer: &UITapGestureRecognizer) {
            let window = self.window().unwrap();

            if recognizer.state() == UIGestureRecognizerState::Ended {
                let gesture_event = EventWrapper::StaticEvent(Event::WindowEvent {
                    window_id: RootWindowId(window.id()),
                    event: WindowEvent::DoubleTapGesture {
                        device_id: DEVICE_ID,
                    },
                });

                let mtm = MainThreadMarker::new().unwrap();
                app_state::handle_nonuser_event(mtm, gesture_event);
            }
        }

        #[method(rotationGesture:)]
        fn rotation_gesture(&self, recognizer: &UIRotationGestureRecognizer) {
            let window = self.window().unwrap();

            let (phase, delta) = match recognizer.state() {
                UIGestureRecognizerState::Began => {
                    self.ivars().rotation_last_delta.set(0.0);

                    (TouchPhase::Started, 0.0)
                }
                UIGestureRecognizerState::Changed => {
                    let last_rotation = self.ivars().rotation_last_delta.replace(recognizer.rotation());

                    (TouchPhase::Moved, recognizer.rotation() - last_rotation)
                }
                UIGestureRecognizerState::Ended => {
                    let last_rotation = self.ivars().rotation_last_delta.replace(0.0);

                    (TouchPhase::Ended, recognizer.rotation() - last_rotation)
                }
                UIGestureRecognizerState::Cancelled | UIGestureRecognizerState::Failed => {
                    self.ivars().rotation_last_delta.set(0.0);

                    // Pass -delta so that action is reversed
                    (TouchPhase::Cancelled, -recognizer.rotation())
                }
                state => panic!("unexpected recognizer state: {state:?}"),
            };

            // Make delta negative to match macos, convert to degrees
            let gesture_event = EventWrapper::StaticEvent(Event::WindowEvent {
                window_id: RootWindowId(window.id()),
                event: WindowEvent::RotationGesture {
                    device_id: DEVICE_ID,
                    delta: -delta.to_degrees() as _,
                    phase,
                },
            });

            let mtm = MainThreadMarker::new().unwrap();
            app_state::handle_nonuser_event(mtm, gesture_event);
        }

        #[method(panGesture:)]
        fn pan_gesture(&self, recognizer: &UIPanGestureRecognizer) {
            let window = self.window().unwrap();

            let translation = recognizer.translationInView(Some(self));

            let (phase, dx, dy) = match recognizer.state() {
                UIGestureRecognizerState::Began => {
                    self.ivars().pan_last_delta.set(translation);

                    (TouchPhase::Started, 0.0, 0.0)
                }
                UIGestureRecognizerState::Changed => {
                    let last_pan: CGPoint = self.ivars().pan_last_delta.replace(translation);

                    let dx = translation.x - last_pan.x;
                    let dy = translation.y - last_pan.y;

                    (TouchPhase::Moved, dx, dy)
                }
                UIGestureRecognizerState::Ended => {
                    let last_pan: CGPoint = self.ivars().pan_last_delta.replace(CGPoint{x:0.0, y:0.0});

                    let dx = translation.x - last_pan.x;
                    let dy = translation.y - last_pan.y;

                    (TouchPhase::Ended, dx, dy)
                }
                UIGestureRecognizerState::Cancelled | UIGestureRecognizerState::Failed => {
                    let last_pan: CGPoint = self.ivars().pan_last_delta.replace(CGPoint{x:0.0, y:0.0});

                    // Pass -delta so that action is reversed
                    (TouchPhase::Cancelled, -last_pan.x, -last_pan.y)
                }
                state => panic!("unexpected recognizer state: {state:?}"),
            };


            let gesture_event = EventWrapper::StaticEvent(Event::WindowEvent {
                window_id: RootWindowId(window.id()),
                event: WindowEvent::PanGesture {
                    device_id: DEVICE_ID,
                    delta: PhysicalPosition::new(dx as _, dy as _),
                    phase,
                },
            });

            let mtm = MainThreadMarker::new().unwrap();
            app_state::handle_nonuser_event(mtm, gesture_event);
        }

        // SolidCraft: a hovering Apple Pencil (or a trackpad's pointer) moves the cursor.
        #[method(hoverGesture:)]
        fn hover_gesture(&self, recognizer: &UIHoverGestureRecognizer) {
            self.handle_hover(recognizer)
        }

        // SolidCraft: the Apple Pencil's own gestures, which are no touches: a double tap on its
        // side and (Apple Pencil Pro) a squeeze. winit has no event for either, so they are
        // reported as function keys that no iPad keyboard sends through this view (see
        // `pencil_key`): F13 pressed and released is a double tap, F14 is down for as long as the
        // squeeze lasts, and F15 pressed and released says the system took the squeeze back.
        //
        // These are `UIPencilInteractionDelegate`'s methods (iPadOS 17.5). They are declared
        // here and not in the protocol's block below, where a build with debug assertions
        // refuses a method the running system's protocol does not have yet.
        #[method(pencilInteraction:didReceiveTap:)]
        fn pencil_did_receive_tap(&self, _interaction: &UIPencilInteraction, _tap: &UIPencilInteractionTap) {
            self.pencil_key(NamedKey::F13, KeyCode::F13, ElementState::Pressed);
            self.pencil_key(NamedKey::F13, KeyCode::F13, ElementState::Released);
        }

        #[method(pencilInteraction:didReceiveSqueeze:)]
        fn pencil_did_receive_squeeze(&self, _interaction: &UIPencilInteraction, squeeze: &UIPencilInteractionSqueeze) {
            self.handle_pencil_squeeze(squeeze)
        }

        #[method(canBecomeFirstResponder)]
        fn can_become_first_responder(&self) -> bool {
            true
        }
    }

    unsafe impl NSObjectProtocol for WinitView {}

    unsafe impl UIGestureRecognizerDelegate for WinitView {
        #[method(gestureRecognizer:shouldRecognizeSimultaneouslyWithGestureRecognizer:)]
        fn should_recognize_simultaneously(&self, _gesture_recognizer: &UIGestureRecognizer, _other_gesture_recognizer: &UIGestureRecognizer) -> bool {
            true
        }
    }

    // SolidCraft: the delegate of the view's `UIPencilInteraction`. Its two methods are declared
    // with the view's own above (see `pencil_did_receive_tap`).
    unsafe impl UIPencilInteractionDelegate for WinitView {}

    unsafe impl UITextInputTraits for WinitView {
    }

    unsafe impl UIKeyInput for WinitView {
        #[method(hasText)]
        fn has_text(&self) -> bool {
            true
        }

        #[method(insertText:)]
        fn insert_text(&self, text: &NSString) {
            self.handle_insert_text(text)
        }

        #[method(deleteBackward)]
        fn delete_backward(&self) {
            self.handle_delete_backward()
        }
    }
);

impl WinitView {
    pub(crate) fn new(
        mtm: MainThreadMarker,
        window_attributes: &WindowAttributes,
        frame: CGRect,
    ) -> Retained<Self> {
        let this = mtm.alloc().set_ivars(WinitViewState {
            pinch_gesture_recognizer: RefCell::new(None),
            doubletap_gesture_recognizer: RefCell::new(None),
            rotation_gesture_recognizer: RefCell::new(None),
            pan_gesture_recognizer: RefCell::new(None),

            rotation_last_delta: Cell::new(0.0),
            pinch_last_delta: Cell::new(0.0),
            pan_last_delta: Cell::new(CGPoint { x: 0.0, y: 0.0 }),

            touches_down: Cell::new(0),
        });
        let this: Retained<Self> = unsafe { msg_send_id![super(this), initWithFrame: frame] };

        this.setMultipleTouchEnabled(true);
        // SolidCraft: always on, unlike the finger gestures above.
        this.recognize_pencil();

        if let Some(scale_factor) = window_attributes.platform_specific.scale_factor {
            this.setContentScaleFactor(scale_factor as _);
        }

        this
    }

    fn window(&self) -> Option<Retained<WinitUIWindow>> {
        // SAFETY: `WinitView`s are always installed in a `WinitUIWindow`
        (**self).window().map(|window| unsafe { Retained::cast(window) })
    }

    pub(crate) fn recognize_pinch_gesture(&self, should_recognize: bool) {
        let mtm = MainThreadMarker::from(self);
        if should_recognize {
            if self.ivars().pinch_gesture_recognizer.borrow().is_none() {
                let pinch = unsafe {
                    UIPinchGestureRecognizer::initWithTarget_action(
                        mtm.alloc(),
                        Some(self),
                        Some(sel!(pinchGesture:)),
                    )
                };
                pinch.setDelegate(Some(ProtocolObject::from_ref(self)));
                self.addGestureRecognizer(&pinch);
                self.ivars().pinch_gesture_recognizer.replace(Some(pinch));
            }
        } else if let Some(recognizer) = self.ivars().pinch_gesture_recognizer.take() {
            self.removeGestureRecognizer(&recognizer);
        }
    }

    pub(crate) fn recognize_pan_gesture(
        &self,
        should_recognize: bool,
        minimum_number_of_touches: u8,
        maximum_number_of_touches: u8,
    ) {
        let mtm = MainThreadMarker::from(self);
        if should_recognize {
            if self.ivars().pan_gesture_recognizer.borrow().is_none() {
                let pan = unsafe {
                    UIPanGestureRecognizer::initWithTarget_action(
                        mtm.alloc(),
                        Some(self),
                        Some(sel!(panGesture:)),
                    )
                };
                pan.setDelegate(Some(ProtocolObject::from_ref(self)));
                pan.setMinimumNumberOfTouches(minimum_number_of_touches as _);
                pan.setMaximumNumberOfTouches(maximum_number_of_touches as _);
                self.addGestureRecognizer(&pan);
                self.ivars().pan_gesture_recognizer.replace(Some(pan));
            }
        } else if let Some(recognizer) = self.ivars().pan_gesture_recognizer.take() {
            self.removeGestureRecognizer(&recognizer);
        }
    }

    pub(crate) fn recognize_doubletap_gesture(&self, should_recognize: bool) {
        let mtm = MainThreadMarker::from(self);
        if should_recognize {
            if self.ivars().doubletap_gesture_recognizer.borrow().is_none() {
                let tap = unsafe {
                    UITapGestureRecognizer::initWithTarget_action(
                        mtm.alloc(),
                        Some(self),
                        Some(sel!(doubleTapGesture:)),
                    )
                };
                tap.setDelegate(Some(ProtocolObject::from_ref(self)));
                tap.setNumberOfTapsRequired(2);
                tap.setNumberOfTouchesRequired(1);
                self.addGestureRecognizer(&tap);
                self.ivars().doubletap_gesture_recognizer.replace(Some(tap));
            }
        } else if let Some(recognizer) = self.ivars().doubletap_gesture_recognizer.take() {
            self.removeGestureRecognizer(&recognizer);
        }
    }

    pub(crate) fn recognize_rotation_gesture(&self, should_recognize: bool) {
        let mtm = MainThreadMarker::from(self);
        if should_recognize {
            if self.ivars().rotation_gesture_recognizer.borrow().is_none() {
                let rotation = unsafe {
                    UIRotationGestureRecognizer::initWithTarget_action(
                        mtm.alloc(),
                        Some(self),
                        Some(sel!(rotationGesture:)),
                    )
                };
                rotation.setDelegate(Some(ProtocolObject::from_ref(self)));
                self.addGestureRecognizer(&rotation);
                self.ivars().rotation_gesture_recognizer.replace(Some(rotation));
            }
        } else if let Some(recognizer) = self.ivars().rotation_gesture_recognizer.take() {
            self.removeGestureRecognizer(&recognizer);
        }
    }

    /// SolidCraft: report a hovering Apple Pencil (or a trackpad's pointer) as the cursor, and the
    /// Pencil's double tap and squeeze as keys (see `pencil_did_receive_tap`).
    fn recognize_pencil(&self) {
        let mtm = MainThreadMarker::from(self);
        let hover = unsafe {
            UIHoverGestureRecognizer::initWithTarget_action(
                mtm.alloc(),
                Some(self),
                Some(sel!(hoverGesture:)),
            )
        };
        hover.setDelegate(Some(ProtocolObject::from_ref(self)));
        self.addGestureRecognizer(&hover);

        // The class is there from iPadOS 12.1, the squeeze from 17.5.
        if AnyClass::get("UIPencilInteraction").is_some() {
            let pencil = unsafe { UIPencilInteraction::new(mtm) };
            unsafe { pencil.setDelegate(Some(ProtocolObject::from_ref(self))) };
            unsafe { self.addInteraction(ProtocolObject::from_ref(&*pencil)) };
        }
    }

    /// SolidCraft: where a point of this view is in the window's pixels, as a touch's is.
    fn physical_position(&self, logical: CGPoint) -> PhysicalPosition<f64> {
        let scale_factor = self.contentScaleFactor();
        PhysicalPosition::from_logical::<(f64, f64), f64>(
            (logical.x as _, logical.y as _),
            scale_factor as f64,
        )
    }

    /// SolidCraft: see `hover_gesture`.
    fn handle_hover(&self, recognizer: &UIHoverGestureRecognizer) {
        // In contact the Pencil is a touch, and the hover that ends as its tip lands must not
        // take the pointer away from that touch.
        if self.ivars().touches_down.get() > 0 {
            return;
        }
        let Some(window) = self.window() else {
            return;
        };
        let event = match recognizer.state() {
            UIGestureRecognizerState::Began | UIGestureRecognizerState::Changed => {
                let location = unsafe { recognizer.locationInView(None) };
                WindowEvent::CursorMoved {
                    device_id: DEVICE_ID,
                    position: self.physical_position(location),
                }
            },
            _ => WindowEvent::CursorLeft { device_id: DEVICE_ID },
        };
        let mtm = MainThreadMarker::from(self);
        app_state::handle_nonuser_event(
            mtm,
            EventWrapper::StaticEvent(Event::WindowEvent {
                window_id: RootWindowId(window.id()),
                event,
            }),
        );
    }

    /// SolidCraft: see `pencil_did_receive_tap`.
    fn handle_pencil_squeeze(&self, squeeze: &UIPencilInteractionSqueeze) {
        let phase = unsafe { squeeze.phase() };
        if phase == UIPencilInteractionPhase::Began {
            // Where the tip is, for an app that heard no hover before the squeeze (the Pencil
            // came into range already squeezed, or hover is switched off).
            if let (Some(pose), Some(window)) = (unsafe { squeeze.hoverPose() }, self.window()) {
                let location = unsafe { pose.location() };
                let mtm = MainThreadMarker::from(self);
                app_state::handle_nonuser_event(
                    mtm,
                    EventWrapper::StaticEvent(Event::WindowEvent {
                        window_id: RootWindowId(window.id()),
                        event: WindowEvent::CursorMoved {
                            device_id: DEVICE_ID,
                            position: self.physical_position(location),
                        },
                    }),
                );
            }
            self.pencil_key(NamedKey::F14, KeyCode::F14, ElementState::Pressed);
        } else if phase == UIPencilInteractionPhase::Ended {
            self.pencil_key(NamedKey::F14, KeyCode::F14, ElementState::Released);
        } else if phase == UIPencilInteractionPhase::Cancelled {
            self.pencil_key(NamedKey::F15, KeyCode::F15, ElementState::Pressed);
            self.pencil_key(NamedKey::F15, KeyCode::F15, ElementState::Released);
            self.pencil_key(NamedKey::F14, KeyCode::F14, ElementState::Released);
        }
    }

    /// SolidCraft: one key event for a Pencil gesture.
    fn pencil_key(&self, key: NamedKey, code: KeyCode, state: ElementState) {
        let Some(window) = self.window() else {
            return;
        };
        let mtm = MainThreadMarker::from(self);
        app_state::handle_nonuser_event(
            mtm,
            EventWrapper::StaticEvent(Event::WindowEvent {
                window_id: RootWindowId(window.id()),
                event: WindowEvent::KeyboardInput {
                    device_id: DEVICE_ID,
                    event: KeyEvent {
                        state,
                        logical_key: Key::Named(key),
                        physical_key: PhysicalKey::Code(code),
                        platform_specific: KeyEventExtra {},
                        repeat: false,
                        location: KeyLocation::Standard,
                        text: None,
                    },
                    is_synthetic: false,
                },
            }),
        );
    }

    fn handle_touches(&self, touches: &NSSet<UITouch>) {
        let window = self.window().unwrap();
        let mut touch_events = Vec::new();
        let os_supports_force = app_state::os_capabilities().force_touch;
        for touch in touches {
            let logical_location = touch.locationInView(None);
            let touch_type = touch.r#type();
            let force = if os_supports_force {
                let trait_collection = self.traitCollection();
                let touch_capability = trait_collection.forceTouchCapability();
                // Both the OS _and_ the device need to be checked for force touch support.
                if touch_capability == UIForceTouchCapability::Available
                    || touch_type == UITouchType::Pencil
                {
                    let force = touch.force();
                    let max_possible_force = touch.maximumPossibleForce();
                    let altitude_angle: Option<f64> = if touch_type == UITouchType::Pencil {
                        let angle = touch.altitudeAngle();
                        Some(angle as _)
                    } else {
                        None
                    };
                    Some(Force::Calibrated {
                        force: force as _,
                        max_possible_force: max_possible_force as _,
                        altitude_angle,
                    })
                } else {
                    None
                }
            } else {
                None
            };
            let touch_id = touch as *const UITouch as u64;
            let phase = touch.phase();
            let phase = match phase {
                UITouchPhase::Began => TouchPhase::Started,
                UITouchPhase::Moved => TouchPhase::Moved,
                // 2 is UITouchPhase::Stationary and is not expected here
                UITouchPhase::Ended => TouchPhase::Ended,
                UITouchPhase::Cancelled => TouchPhase::Cancelled,
                _ => panic!("unexpected touch phase: {phase:?}"),
            };
            // SolidCraft: see `touches_down`.
            let down = &self.ivars().touches_down;
            match phase {
                TouchPhase::Started => down.set(down.get() + 1),
                TouchPhase::Ended | TouchPhase::Cancelled => down.set(down.get().saturating_sub(1)),
                TouchPhase::Moved => {},
            }

            let physical_location = {
                let scale_factor = self.contentScaleFactor();
                PhysicalPosition::from_logical::<(f64, f64), f64>(
                    (logical_location.x as _, logical_location.y as _),
                    scale_factor as f64,
                )
            };
            touch_events.push(EventWrapper::StaticEvent(Event::WindowEvent {
                window_id: RootWindowId(window.id()),
                event: WindowEvent::Touch(Touch {
                    device_id: DEVICE_ID,
                    id: touch_id,
                    location: physical_location,
                    force,
                    phase,
                }),
            }));
        }
        let mtm = MainThreadMarker::new().unwrap();
        app_state::handle_nonuser_events(mtm, touch_events);
    }

    fn handle_insert_text(&self, text: &NSString) {
        let window = self.window().unwrap();
        let window_id = RootWindowId(window.id());
        let mtm = MainThreadMarker::new().unwrap();
        // send individual events for each character
        app_state::handle_nonuser_events(
            mtm,
            text.to_string().chars().flat_map(|c| {
                let text = smol_str::SmolStr::from_iter([c]);
                // Emit both press and release events
                [ElementState::Pressed, ElementState::Released].map(|state| {
                    EventWrapper::StaticEvent(Event::WindowEvent {
                        window_id,
                        event: WindowEvent::KeyboardInput {
                            event: KeyEvent {
                                text: if state == ElementState::Pressed {
                                    Some(text.clone())
                                } else {
                                    None
                                },
                                state,
                                location: KeyLocation::Standard,
                                repeat: false,
                                logical_key: Key::Character(text.clone()),
                                physical_key: PhysicalKey::Unidentified(
                                    NativeKeyCode::Unidentified,
                                ),
                                platform_specific: KeyEventExtra {},
                            },
                            is_synthetic: false,
                            device_id: DEVICE_ID,
                        },
                    })
                })
            }),
        );
    }

    fn handle_delete_backward(&self) {
        let window = self.window().unwrap();
        let window_id = RootWindowId(window.id());
        let mtm = MainThreadMarker::new().unwrap();
        app_state::handle_nonuser_events(
            mtm,
            [ElementState::Pressed, ElementState::Released].map(|state| {
                EventWrapper::StaticEvent(Event::WindowEvent {
                    window_id,
                    event: WindowEvent::KeyboardInput {
                        device_id: DEVICE_ID,
                        event: KeyEvent {
                            state,
                            logical_key: Key::Named(NamedKey::Backspace),
                            physical_key: PhysicalKey::Code(KeyCode::Backspace),
                            platform_specific: KeyEventExtra {},
                            repeat: false,
                            location: KeyLocation::Standard,
                            text: None,
                        },
                        is_synthetic: false,
                    },
                })
            }),
        );
    }
}
