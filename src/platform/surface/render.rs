//! Markdown-lite as an `NSAttributedString`: the runs `style` makes from `core::markup`, with
//! system fonts (semibold for bold, the italic trait for italic, monospaced for code), a
//! shaded background behind code, paragraph indents and spacing, and the link attribute on
//! `http(s)` links only. Used by surface bubbles and HUD card text blocks.

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{AnyThread, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSAttributedStringNSExtendedStringDrawing, NSBackgroundColorAttributeName, NSColor, NSFont,
    NSFontAttributeName, NSFontDescriptorSymbolicTraits, NSFontWeightRegular, NSFontWeightSemibold,
    NSForegroundColorAttributeName, NSLinkAttributeName, NSMutableParagraphStyle,
    NSParagraphStyleAttributeName, NSStringDrawingOptions, NSTextView,
};
use objc2_foundation::{
    NSAttributedString, NSAttributedStringKey, NSDictionary, NSMutableAttributedString, NSRect,
    NSSize, NSString, NSURL,
};

use super::style::{self, Font, Run};

fn font(f: Font) -> Retained<NSFont> {
    // SAFETY: the weights are immutable framework constants, set at load time.
    let weight = unsafe { if f.bold { NSFontWeightSemibold } else { NSFontWeightRegular } };
    let base = if f.mono {
        NSFont::monospacedSystemFontOfSize_weight(f.size, weight)
    } else {
        NSFont::systemFontOfSize_weight(f.size, weight)
    };
    if !f.italic {
        return base;
    }
    let traits =
        base.fontDescriptor().symbolicTraits() | NSFontDescriptorSymbolicTraits::TraitItalic;
    let italic = base.fontDescriptor().fontDescriptorWithSymbolicTraits(traits);
    NSFont::fontWithDescriptor_size(&italic, f.size).unwrap_or(base)
}

fn run_string(run: &Run, color: &NSColor) -> Retained<NSAttributedString> {
    let para = NSMutableParagraphStyle::new();
    para.setFirstLineHeadIndent(run.para.first);
    para.setHeadIndent(run.para.rest);
    para.setParagraphSpacingBefore(run.para.before);
    let font = font(run.font);
    // SAFETY: the attribute keys are immutable framework constants, set at load time.
    let mut keys: Vec<&NSAttributedStringKey> = unsafe {
        vec![NSFontAttributeName, NSForegroundColorAttributeName, NSParagraphStyleAttributeName]
    };
    let mut values: Vec<&AnyObject> = vec![font.as_ref(), color.as_ref(), para.as_ref()];
    let shade = NSColor::unemphasizedSelectedTextBackgroundColor();
    if run.code {
        // SAFETY: as above.
        keys.push(unsafe { NSBackgroundColorAttributeName });
        values.push(shade.as_ref());
    }
    let url = run.link.as_deref().and_then(|l| NSURL::URLWithString(&NSString::from_str(l)));
    if let Some(url) = &url {
        // SAFETY: as above.
        keys.push(unsafe { NSLinkAttributeName });
        values.push(url.as_ref());
    }
    let attrs = NSDictionary::from_slices(&keys, &values);
    // SAFETY: each attribute holds the class AppKit expects for its key (NSFont, NSColor,
    // NSParagraphStyle, NSURL).
    unsafe {
        NSAttributedString::initWithString_attributes(
            NSAttributedString::alloc(),
            &NSString::from_str(&run.text),
            Some(&attrs),
        )
    }
}

/// `md` at body `size` in `color` (links keep the system link color).
pub(in crate::platform) fn attributed(
    md: &str,
    size: f64,
    color: &NSColor,
) -> Retained<NSAttributedString> {
    let out = NSMutableAttributedString::new();
    for run in style::runs(md, size) {
        out.appendAttributedString(&run_string(&run, color));
    }
    out.into_super()
}

/// A borderless, non-editable text view showing `string` edge to edge; `selectable` lets the
/// user select and copy it and click its links.
pub(in crate::platform) fn text_view(
    mtm: MainThreadMarker,
    string: &NSAttributedString,
    selectable: bool,
) -> Retained<NSTextView> {
    let view = NSTextView::initWithFrame(NSTextView::alloc(mtm), NSRect::ZERO);
    view.setEditable(false);
    view.setSelectable(selectable);
    view.setRichText(true);
    view.setDrawsBackground(false);
    view.setTextContainerInset(NSSize::ZERO);
    // SAFETY: the text view's own container, used on the main thread.
    if let Some(c) = unsafe { view.textContainer() } {
        c.setLineFragmentPadding(0.0);
    }
    // SAFETY: the text view's own storage, used on the main thread.
    if let Some(storage) = unsafe { view.textStorage() } {
        storage.setAttributedString(string);
    }
    view
}

/// The size of `string` wrapped at `max` points, rounded up (at most `max` wide).
pub(in crate::platform) fn measure(string: &NSAttributedString, max: f64) -> (f64, f64) {
    let r = string.boundingRectWithSize_options_context(
        NSSize::new(max, 1.0e7),
        NSStringDrawingOptions::UsesLineFragmentOrigin,
        None,
    );
    ((r.size.width.ceil() + 1.0).min(max), r.size.height.ceil() + 1.0)
}
