use gpui::prelude::*;
use gpui::{App, Hsla, Pixels, px};

use crate::theme::{ActiveTheme as _, Design, GlassStyle};

/// How much of the popover colour a glass control keeps over what it blurs.
pub const GLASS_FILL: f32 = 0.2;
/// How hard a glass control blurs what is behind it.
pub const GLASS_BLUR: Pixels = px(8.);
/// Raised surfaces retain contrast even over bright artwork, while a faint backdrop remains visible.
const POPOVER_FILL: f32 = 0.82;
/// How much of the popover colour a frosted bar keeps, and how far it blurs what passes under.
/// Both are thicker than a menu's, since a bar sits over the page for good and its text has to
/// read over whatever scrolls by.
const BAR_FILL: f32 = 0.86;
const BAR_BLUR: Pixels = px(18.);
/// The bar's fill on a see-through window. The page wash already sits under the bar, so the thick
/// fill would stack on it and leave the bar far darker than the window around it.
const CLEAR_BAR_FILL: f32 = 0.3;

/// A liquid glass bar keeps less of its fill than a plain bar, and blurs harder, so the page
/// shows through it. Frosted keeps more fill than Clear, which lets the rim do the work.
const LIQUID_BAR_FILL: f32 = 0.55;
const LIQUID_BAR_BLUR: Pixels = px(24.);
const LIQUID_BAR_FILL_CLEAR: f32 = 0.3;
const LIQUID_BAR_BLUR_CLEAR: Pixels = px(12.);
const LIQUID_CLEAR_BAR_FILL: f32 = 0.18;
/// A liquid glass control keeps less fill and blurs harder than a plain one.
const LIQUID_FILL: f32 = 0.12;
const LIQUID_BLUR: Pixels = px(12.);
const LIQUID_FILL_CLEAR: f32 = 0.06;
const LIQUID_BLUR_CLEAR: Pixels = px(6.);
/// A liquid popover keeps less fill than a plain one, and blurs harder when frosted. The
/// settings preview cards draw their glass pill from these same values, so the preview shows
/// the real treatment.
pub const LIQUID_POPOVER_FILL: f32 = 0.55;
pub const LIQUID_POPOVER_BLUR: Pixels = px(16.);
pub const LIQUID_POPOVER_FILL_CLEAR: f32 = 0.25;
pub const LIQUID_POPOVER_BLUR_CLEAR: Pixels = px(6.);
/// The corner radius of a liquid surface. Larger than the theme radius, so a liquid menu or
/// card reads as a pill rather than a plain panel.
const LIQUID_RADIUS: Pixels = px(18.);
/// The padding inside a bento box, and the radius of its corners.
pub const BENTO_PAD: Pixels = px(12.);
pub const BENTO_RADIUS: Pixels = px(20.);

/// Whether a surface that only frosts for the look of it should frost at all. A backdrop blur
/// is the priciest thing the renderer does per frame, so the setting turns those off and each
/// one falls back to a flat fill. The settings header and the fullscreen chrome frost whatever
/// it answers: their haze is what keeps text readable over the content passing under it.
pub fn blurring(cx: &App) -> bool {
    cx.theme().blur
}

/// Whether surfaces draw as liquid glass: the design is Liquid Glass and the blur setting is
/// on. Blur stays the master switch, so turning it off flattens every liquid surface back to
/// a flat fill, the way it flattens the plain glass ones.
pub fn liquid_glass(cx: &App) -> bool {
    matches!(cx.theme().design, Design::LiquidGlass) && blurring(cx)
}

/// Whether liquid glass is in its Clear style rather than Frosted. Only read while
/// `liquid_glass` holds.
pub fn liquid_clear(cx: &App) -> bool {
    matches!(cx.theme().glass_style, GlassStyle::Clear)
}

/// The colour of a liquid surface's rim: the popover foreground thinned, so the edge reads
/// bright on a dark theme and dark on a light one, on either side of the glass.
pub fn rim_color(cx: &App) -> Hsla {
    cx.theme().popover_foreground.opacity(0.22)
}

/// The colour of a liquid surface's bright top edge, stronger than the rim around it.
pub fn top_highlight_color(cx: &App) -> Hsla {
    cx.theme().popover_foreground.opacity(0.45)
}

/// Styles an element as frosted glass: a faint popover fill over a hard blur of
/// whatever it floats on. Over a flat colour the blur shows nothing, so the fill is all
/// that separates the control from the page there. A site the blur setting reaches asks
/// `blurring` before calling this, since the thin fill needs the wash to read.
pub fn glass<E: Styled>(element: E, cx: &App) -> E {
    let theme = cx.theme();

    match liquid_glass(cx) {
        true => {
            let (fill, blur) = match liquid_clear(cx) {
                true => (LIQUID_FILL_CLEAR, LIQUID_BLUR_CLEAR),
                false => (LIQUID_FILL, LIQUID_BLUR),
            };
            element
                .bg(theme.popover.opacity(fill))
                .backdrop_blur(blur)
        }
        false => element
            .bg(theme.popover.opacity(GLASS_FILL))
            .backdrop_blur(GLASS_BLUR),
    }
}

/// The fill shared by context menus and raised translucent panels. With blur disabled it
/// becomes opaque, since there is no wash behind the text to preserve its contrast.
pub fn popover_fill(cx: &App) -> Hsla {
    let theme = cx.theme();
    match blurring(cx) {
        true => match liquid_glass(cx) {
            true => theme.popover.opacity(match liquid_clear(cx) {
                true => LIQUID_POPOVER_FILL_CLEAR,
                false => LIQUID_POPOVER_FILL,
            }),
            false => theme.popover.opacity(POPOVER_FILL),
        },
        false => theme.popover,
    }
}

/// Gives a raised panel the same fill, foreground and backdrop blur as a context menu.
/// With liquid glass it thins the fill, blurs harder, and gains a bright rim and a pill
/// radius, so a menu reads as a glass pill over the page.
pub fn popover_surface<E: Styled>(element: E, cx: &App) -> E {
    let element = element
        .bg(popover_fill(cx))
        .text_color(cx.theme().popover_foreground);
    match blurring(cx) {
        true => match liquid_glass(cx) {
            true => {
                let blur = match liquid_clear(cx) {
                    true => LIQUID_POPOVER_BLUR_CLEAR,
                    false => LIQUID_POPOVER_BLUR,
                };
                element
                    .backdrop_blur(blur)
                    .border_1()
                    .border_color(rim_color(cx))
                    .rounded(LIQUID_RADIUS)
            }
            false => frost(element),
        },
        false => element,
    }
}

/// Styles chrome that stays over scrolling content, the player bar and a table's head: a thick
/// popover fill over a wide blur. The caller asks `blurring` first, since without the blur the
/// thin fill would leave the text on the bare page. A see-through window gets a thinner fill so
/// the bar lets the desktop through about as much as the page does. With liquid glass the fill
/// thins further and the blur widens, so the page shows through the bar.
pub fn bar_surface<E: Styled>(element: E, cx: &App) -> E {
    let theme = cx.theme();
    match liquid_glass(cx) {
        true => {
            let (fill, blur) = match theme.transparent {
                true => (LIQUID_CLEAR_BAR_FILL, LIQUID_BAR_BLUR),
                false => match liquid_clear(cx) {
                    true => (LIQUID_BAR_FILL_CLEAR, LIQUID_BAR_BLUR_CLEAR),
                    false => (LIQUID_BAR_FILL, LIQUID_BAR_BLUR),
                },
            };
            element
                .bg(theme.popover.opacity(fill))
                .backdrop_blur(blur)
        }
        false => {
            let fill = match theme.transparent {
                true => CLEAR_BAR_FILL,
                false => BAR_FILL,
            };
            element
                .bg(theme.popover.opacity(fill))
                .backdrop_blur(BAR_BLUR)
        }
    }
}

/// Blurs what is behind an element by the glass width and nothing more, for a hover fill
/// that is already translucent. Only hovers that float over real content go through it;
/// on flat paint the blur shows nothing, so those stay plain fills. The caller asks
/// `blurring` first, since this one has no theme to read.
pub fn frost<E: Styled>(element: E) -> E {
    element.backdrop_blur(GLASS_BLUR)
}

/// Styles an element as a bento box: its own rounded surface with a gap around it. With
/// liquid glass it is a glass pill over the page, with a bright rim; otherwise a plain
/// secondary surface with the theme border. It sets no text colour, so the content inside
/// keeps the page's own colours.
pub fn bento<E: Styled>(element: E, cx: &App) -> E {
    let theme = cx.theme();
    let element = element.rounded(BENTO_RADIUS).border_1();
    match liquid_glass(cx) {
        true => {
            let (fill, blur) = match liquid_clear(cx) {
                true => (LIQUID_POPOVER_FILL_CLEAR, LIQUID_POPOVER_BLUR_CLEAR),
                false => (LIQUID_POPOVER_FILL, LIQUID_POPOVER_BLUR),
            };
            element
                .bg(theme.popover.opacity(fill))
                .backdrop_blur(blur)
                .border_color(rim_color(cx))
        }
        false => element
            .bg(theme.secondary)
            .border_color(theme.border),
    }
}
