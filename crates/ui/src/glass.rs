use gpui::prelude::*;
use gpui::{
    App, Background, BoxShadow, Div, Hsla, InteractiveElement, ObjectFit, Pixels,
    StatefulInteractiveElement, div, img, linear_color_stop, linear_gradient, px,
};

use crate::motion::mix;
use crate::theme::{ActiveTheme as _, Design, GlassStyle};

/// The tiled grain liquid glass lays over its fill, so the surface reads as
/// glass rather than flat plastic. It is registered in the app's assets as
/// `images/noise.png` and drawn at a few percent opacity.
const NOISE: &str = "images/noise.png";
/// How far a liquid fill leans from the neutral popover colour toward the
/// theme's accent, so glass is tinted instead of grey.
const LIQUID_TINT_MIX: f32 = 0.3;
/// The bright top edge of a liquid surface, as a share of the rim colour.
const LIQUID_RIM_TOP: f32 = 0.35;
/// The noise a liquid surface carries, as a share of full strength.
const LIQUID_NOISE: f32 = 0.03;
/// How tall the bright band across a liquid surface's top edge is.
const HIGHLIGHT_H: Pixels = px(2.);
/// How tall the darker wash across a liquid surface's bottom edge is.
const SHADE_H: Pixels = px(10.);
/// How far a liquid surface's soft shadow sits below it, and how far it blurs.
const LIQUID_SHADOW_Y: Pixels = px(8.);
const LIQUID_SHADOW_BLUR: Pixels = px(24.);
/// How much of the shadow colour a liquid surface's soft shadow keeps.
const LIQUID_SHADOW_ALPHA: f32 = 0.25;
/// The hover lift and press dip of a liquid control, as a share of its size.
const LIQUID_HOVER_SCALE: f32 = 1.02;
const LIQUID_PRESS_SCALE: f32 = 0.97;

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
const LIQUID_BAR_FILL: f32 = 0.45;
const LIQUID_BAR_BLUR: Pixels = px(32.);
const LIQUID_BAR_FILL_CLEAR: f32 = 0.22;
const LIQUID_BAR_BLUR_CLEAR: Pixels = px(20.);
const LIQUID_CLEAR_BAR_FILL: f32 = 0.12;
/// A liquid glass control keeps less fill and blurs harder than a plain one.
const LIQUID_FILL: f32 = 0.08;
const LIQUID_BLUR: Pixels = px(20.);
const LIQUID_FILL_CLEAR: f32 = 0.04;
const LIQUID_BLUR_CLEAR: Pixels = px(10.);
/// A liquid popover keeps less fill than a plain one, and blurs harder when frosted. The
/// settings preview cards draw their glass pill from these same values, so the preview shows
/// the real treatment.
pub const LIQUID_POPOVER_FILL: f32 = 0.45;
pub const LIQUID_POPOVER_BLUR: Pixels = px(24.);
pub const LIQUID_POPOVER_FILL_CLEAR: f32 = 0.18;
pub const LIQUID_POPOVER_BLUR_CLEAR: Pixels = px(12.);
/// The corner radius of a liquid surface. Larger than the theme radius, so a liquid menu or
/// card reads as a pill rather than a plain panel.
const LIQUID_RADIUS: Pixels = px(24.);
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

/// The colour of a liquid surface's rim: the popover foreground at half strength, so the edge
/// reads clearly on a dark theme and dark on a light one, on either side of the glass.
pub fn rim_color(cx: &App) -> Hsla {
    cx.theme().popover_foreground.opacity(0.5)
}

/// The colour of a liquid surface's bright top edge, stronger than the rim around it.
pub fn top_highlight_color(cx: &App) -> Hsla {
    cx.theme().popover_foreground.opacity(0.8)
}

/// The fill colour of a liquid surface: the popover colour leaned toward the
/// theme's accent, so glass is tinted instead of grey. The accent is the
/// theme's own primary, which the adaptive theme already washes toward the
/// cover, so a track's art tints its glass.
pub fn liquid_fill_color(cx: &App, fill: f32) -> Hsla {
    let theme = cx.theme();
    mix(theme.popover, theme.primary, LIQUID_TINT_MIX).opacity(fill)
}

/// The soft shadow a liquid surface throws: wide, blurred and dark, so the
/// panel lifts off the page instead of sitting flat on it. The colour is the
/// theme's background driven to black, so no new colour is introduced.
fn liquid_shadow(cx: &App) -> BoxShadow {
    let mut black = cx.theme().background;
    black.l = 0.;
    BoxShadow::new(px(0.), LIQUID_SHADOW_Y, black.opacity(LIQUID_SHADOW_ALPHA))
        .blur_radius(LIQUID_SHADOW_BLUR)
}

/// The bright band, darker foot and grain a liquid panel carries over its
/// fill: a highlight across the top edge fading down, a darker wash across
/// the bottom edge, and a faint tiled noise that keeps the glass from
/// reading as flat plastic. The strips are absolutely positioned, so they
/// never move the panel's own content, and they take the panel's radius so
/// they never poke past its corners.
fn liquid_overlays(element: Div, cx: &App, radius: Pixels) -> Div {
    let theme = cx.theme();
    let mut bottom = theme.popover;
    bottom.l = (bottom.l * 0.8).clamp(0., 1.);
    element
        .relative()
        .child(
            div()
                .absolute()
                .top_0()
                .left_0()
                .right_0()
                .h(HIGHLIGHT_H)
                .rounded_tl(radius)
                .rounded_tr(radius)
                .bg(linear_gradient(
                    180.,
                    linear_color_stop(
                        theme.popover_foreground.opacity(LIQUID_RIM_TOP),
                        0.,
                    ),
                    linear_color_stop(theme.popover_foreground.opacity(0.), 1.),
                )),
        )
        .child(
            div()
                .absolute()
                .bottom_0()
                .left_0()
                .right_0()
                .h(SHADE_H)
                .rounded_bl(radius)
                .rounded_br(radius)
                .bg(linear_gradient(
                    180.,
                    linear_color_stop(bottom.opacity(0.), 0.),
                    linear_color_stop(bottom.opacity(0.3), 1.),
                )),
        )
        .child(
            img(NOISE)
                .absolute()
                .inset_0()
                .size_full()
                .object_fit(ObjectFit::Cover)
                .rounded(radius)
                .opacity(LIQUID_NOISE),
        )
}

/// The sheen on a liquid pill: a top-lit gradient over the same fill, bright at the top
/// fading to the fill, so the surface reads as light catching glass rather than a flat
/// tint. Both stops come from the popover colour, so the sheen follows the theme. The
/// settings preview draws its pill from this too, so the preview shows the real treatment.
/// The foot of the gradient is the popover darkened, so the pill carries depth at the
/// bottom as well as light at the top.
pub fn liquid_sheen(cx: &App, fill: f32) -> Background {
    let popover = cx.theme().popover;
    let mut bottom = popover;
    bottom.l = (bottom.l * 0.8).clamp(0., 1.);
    linear_gradient(
        180.,
        linear_color_stop(popover.opacity((fill + 0.3).min(1.)), 0.),
        linear_color_stop(bottom.opacity(fill), 1.),
    )
}

/// The hover lift and press dip of a liquid control: a slight scale up under
/// the pointer and a slight dip while pressed, so glass feels alive rather
/// than painted on. The scale is a paint-only transform, so layout never
/// moves, and GPUI applies state styles at once — the ease comes from the
/// springs the controls already ride, not from this helper.
pub fn liquid_motion<E: Styled + InteractiveElement + StatefulInteractiveElement>(
    element: E,
    cx: &App,
) -> E {
    match liquid_glass(cx) {
        true => element
            .hover(|style| style.layer_scale(LIQUID_HOVER_SCALE))
            .active(|style| style.layer_scale(LIQUID_PRESS_SCALE)),
        false => element,
    }
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
                .bg(liquid_fill_color(cx, fill))
                .backdrop_blur(blur)
                .border_1()
                .border_color(rim_color(cx))
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
            true => liquid_fill_color(
                cx,
                match liquid_clear(cx) {
                    true => LIQUID_POPOVER_FILL_CLEAR,
                    false => LIQUID_POPOVER_FILL,
                },
            ),
            false => theme.popover.opacity(POPOVER_FILL),
        },
        false => theme.popover,
    }
}

/// Gives a raised panel the same fill, foreground and backdrop blur as a context menu.
/// With liquid glass it thins the fill, blurs harder, and gains a bright rim, a top-lit
/// sheen and a pill radius, so a menu reads as a glass pill over the page. It also
/// carries the liquid overlays — the bright top edge, the darker foot and the grain —
/// and a soft shadow, so the panel lifts off the page.
pub fn popover_surface(element: Div, cx: &App) -> Div {
    let element = element
        .bg(popover_fill(cx))
        .text_color(cx.theme().popover_foreground);
    match blurring(cx) {
        true => match liquid_glass(cx) {
            true => {
                let (fill, blur) = match liquid_clear(cx) {
                    true => (LIQUID_POPOVER_FILL_CLEAR, LIQUID_POPOVER_BLUR_CLEAR),
                    false => (LIQUID_POPOVER_FILL, LIQUID_POPOVER_BLUR),
                };
                liquid_overlays(
                    element
                        .bg(liquid_sheen(cx, fill))
                        .backdrop_blur(blur)
                        .border_1()
                        .border_color(rim_color(cx))
                        .rounded(LIQUID_RADIUS)
                        .shadow(vec![liquid_shadow(cx)]),
                    cx,
                    LIQUID_RADIUS,
                )
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
/// thins further and the blur widens, so the page shows through the bar. The fill leans toward
/// the theme's accent, and the bar carries the liquid overlays and a soft shadow.
pub fn bar_surface(element: Div, cx: &App) -> Div {
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
            liquid_overlays(
                element
                    .bg(liquid_fill_color(cx, fill))
                    .backdrop_blur(blur)
                    .shadow(vec![liquid_shadow(cx)]),
                cx,
                px(0.),
            )
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
/// keeps the page's own colours. With liquid glass it also carries the liquid overlays
/// and a soft shadow.
pub fn bento(element: Div, cx: &App) -> Div {
    let theme = cx.theme();
    let element = element.rounded(BENTO_RADIUS).border_1();
    match liquid_glass(cx) {
        true => {
            let (fill, blur) = match liquid_clear(cx) {
                true => (LIQUID_POPOVER_FILL_CLEAR, LIQUID_POPOVER_BLUR_CLEAR),
                false => (LIQUID_POPOVER_FILL, LIQUID_POPOVER_BLUR),
            };
            liquid_overlays(
                element
                    .bg(liquid_sheen(cx, fill))
                    .backdrop_blur(blur)
                    .border_color(rim_color(cx))
                    .shadow(vec![liquid_shadow(cx)]),
                cx,
                BENTO_RADIUS,
            )
        }
        false => element
            .bg(theme.secondary)
            .border_color(theme.border),
    }
}
