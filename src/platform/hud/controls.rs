//! The interactive pieces of a card: controls that take the first click in a panel that is
//! not key, the one target their actions go to (and the delegate of its text fields), and the
//! live controls of one drawn card, read back into `core::card` values at press time.

use std::cell::OnceCell;

use objc2::rc::Retained;
use objc2::runtime::Sel;
use objc2::{MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSButton, NSControl, NSControlStateValueOn, NSControlTextEditingDelegate, NSEvent,
    NSPopUpButton, NSResponder, NSStandardKeyBindingResponding, NSTextField, NSTextFieldDelegate,
    NSTextView, NSView,
};
use objc2_foundation::{NSObject, NSObjectProtocol};

use super::card_layout::CANCEL;
use crate::core::card::{Card, Input};
use crate::platform::timer;

/// The tag of a multiline field: Return inserts a newline.
pub const MULTILINE: isize = 1;
/// The tags of the confirm step's buttons; action buttons are tagged by their index.
pub const RUN_TAG: isize = 1000;
pub const CANCEL_TAG: isize = 1001;

define_class!(
    // A button (push, radio or checkbox) that acts on the first click, though its panel is
    // never key for it.
    #[unsafe(super(NSButton, NSControl, NSView, NSResponder, NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "FlickHudButton"]
    pub struct Button;

    impl Button {
        #[unsafe(method(acceptsFirstMouse:))]
        fn accepts_first_mouse(&self, _event: Option<&NSEvent>) -> bool {
            true
        }
    }
);

define_class!(
    // A pop-up button that opens on the first click.
    #[unsafe(super(NSPopUpButton, NSButton, NSControl, NSView, NSResponder, NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "FlickHudPopUp"]
    pub struct PopUp;

    impl PopUp {
        #[unsafe(method(acceptsFirstMouse:))]
        fn accepts_first_mouse(&self, _event: Option<&NSEvent>) -> bool {
            true
        }
    }
);

define_class!(
    // A text field that takes the first click; clicking it makes the card panel key (it needs
    // the panel to become key), without activating Flick.
    #[unsafe(super(NSTextField, NSControl, NSView, NSResponder, NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "FlickHudField"]
    pub struct Field;

    impl Field {
        #[unsafe(method(acceptsFirstMouse:))]
        fn accepts_first_mouse(&self, _event: Option<&NSEvent>) -> bool {
            true
        }
    }
);

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "FlickHudCardTarget"]
    pub struct Target;

    // SAFETY: the protocols' methods are optional; the ones below have their exact signatures.
    unsafe impl NSObjectProtocol for Target {}
    // SAFETY: as above.
    unsafe impl NSControlTextEditingDelegate for Target {}
    // SAFETY: as above.
    unsafe impl NSTextFieldDelegate for Target {}

    impl Target {
        // A button press. Deferred: the module may redraw the card, removing the sender while
        // AppKit is still delivering its action.
        #[unsafe(method(press:))]
        fn press(&self, sender: &NSButton) {
            let tag = sender.tag();
            if let Some(window) = sender.window() {
                let window = Retained::as_ptr(&window).cast::<()>() as usize;
                timer::after(0.0, move || super::pressed(window, tag));
            }
        }

        // Radio buttons group by superview and action; picking one needs nothing else.
        #[unsafe(method(choose:))]
        fn choose(&self, _sender: &NSButton) {}

        // Return in a multiline field inserts a newline instead of ending the edit.
        #[unsafe(method(control:textView:doCommandBySelector:))]
        fn do_command(&self, control: &NSControl, view: &NSTextView, command: Sel) -> bool {
            let newline = command == sel!(insertNewline:) && control.tag() == MULTILINE;
            if newline {
                // SAFETY: NSTextView implements this action; nil is a valid sender.
                unsafe { view.insertNewlineIgnoringFieldEditor(None) };
            }
            newline
        }
    }
);

thread_local! {
    /// Controls hold their target and delegate weakly; this keeps the one target alive.
    static TARGET: OnceCell<Retained<Target>> = const { OnceCell::new() };
}

/// The target and text-field delegate of every card control.
pub fn target(mtm: MainThreadMarker) -> Retained<Target> {
    TARGET.with(|t| {
        t.get_or_init(|| {
            // SAFETY: plain `init` on a freshly allocated `NSObject` subclass with no ivars.
            unsafe { msg_send![Target::alloc(mtm), init] }
        })
        .clone()
    })
}

/// One input's live control.
pub enum Ctl {
    Text(Retained<Field>),
    /// Option ids in item order, after the `NO_CHOICE` item when `blank`.
    Popup {
        view: Retained<PopUp>,
        ids: Vec<String>,
        blank: bool,
    },
    Radios(Vec<(String, Retained<Button>)>),
    Checks(Vec<(String, Retained<Button>)>),
}

fn on(b: &NSButton) -> bool {
    b.state() == NSControlStateValueOn
}

impl Ctl {
    fn value(&self) -> Input {
        match self {
            Ctl::Text(f) => Input::Text(f.stringValue().to_string()),
            Ctl::Popup { view, ids, blank } => {
                let i = usize::try_from(view.indexOfSelectedItem()).ok();
                let i = i.and_then(|i| if *blank { i.checked_sub(1) } else { Some(i) });
                Input::One(i.and_then(|i| ids.get(i).cloned()))
            }
            Ctl::Radios(bs) => Input::One(bs.iter().find(|(_, b)| on(b)).map(|(id, _)| id.clone())),
            Ctl::Checks(bs) => {
                Input::Many(bs.iter().filter(|(_, b)| on(b)).map(|(id, _)| id.clone()).collect())
            }
        }
    }
}

/// The live controls of one drawn card, and what it was drawn from.
pub struct Controls {
    /// Inputs in card order.
    pub inputs: Vec<(String, Ctl)>,
    /// What the inputs started with at this draw (`card_layout::carry` compares them).
    pub initial: Vec<(String, Input)>,
    /// Action ids, indexed by button tag.
    pub actions: Vec<String>,
    /// The shell action the confirm step's Run presses.
    pub confirm: Option<String>,
    /// The card and its module state as drawn, to redraw it with a local error.
    pub card: Card,
    pub pending: bool,
}

impl Controls {
    /// Every input's current value, in card order.
    pub fn values(&self) -> Vec<(String, Input)> {
        self.inputs.iter().map(|(id, c)| (id.clone(), c.value())).collect()
    }

    /// The action id a button with `tag` presses: an action, the confirmed shell action
    /// (Run) or `CANCEL`.
    pub fn action(&self, tag: isize) -> Option<String> {
        match tag {
            RUN_TAG => self.confirm.clone(),
            CANCEL_TAG => self.confirm.as_ref().map(|_| CANCEL.to_string()),
            _ => usize::try_from(tag).ok().and_then(|i| self.actions.get(i).cloned()),
        }
    }
}
