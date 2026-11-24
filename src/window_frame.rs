//! The window frame on Linux when the app draws its own decorations (Wayland
//! compositors without server-side decorations, X11 with a compositor):
//! a shadow, a border, rounded corners and resize edges, as in Zed's
//! `client_side_decorations`. On macOS, and when the window manager draws the
//! frame, the content is returned as it is.

use gpui::{
    AnyElement, App, Bounds, BoxShadow, Corners, CursorStyle, Decorations, HitboxBehavior, Hsla,
    IntoElement, MouseButton, Pixels, Point, ResizeEdge, Size, Tiling, Window, canvas, div, point,
    prelude::*, px, size,
};

use crate::ui::ActiveTheme as _;

#[cfg(windows)]
pub fn windows_controls(window: &Window, cx: &App) -> Option<gpui::Div> {
    use gpui::{AccessibleAction, ClickEvent, Role, WindowControlArea};
    use windows::Win32::UI::WindowsAndMessaging::{SC_CLOSE, SC_MAXIMIZE, SC_MINIMIZE, SC_RESTORE};

    use crate::ui::{Color, Icon, IconName, IconSize, Tooltip, h_flex};

    if window.is_fullscreen() {
        return None;
    }
    let colors = &cx.theme().colors;
    let hover = colors.ghost_element_hover;
    let active = colors.ghost_element_active;
    let button = |id, area, icon, label, command| {
        div()
            .id(id)
            .role(Role::Button)
            .aria_label(label)
            .tab_index(0)
            .flex()
            .items_center()
            .justify_center()
            .size(px(32.))
            .rounded(px(5.))
            .hover(|style| style.bg(hover))
            .focus_visible(|style| style.bg(hover))
            .active(|style| style.bg(active))
            .window_control_area(area)
            .tooltip(Tooltip::text(label))
            // 鼠标由 GPUI 的 Windows 非客户区处理，键盘和读屏需单独投递系统命令。
            .on_click(move |event, window, _| {
                if let ClickEvent::Keyboard(_) = event {
                    post_window_command(window, command);
                }
            })
            .on_a11y_action(AccessibleAction::Click, move |_, window, _| {
                post_window_command(window, command);
            })
            .child(Icon::new(icon).size(IconSize::Small).color(Color::Default))
    };
    let (max_icon, max_label, max_command) = if window.is_maximized() {
        (IconName::Restore, "Restore", SC_RESTORE)
    } else {
        (IconName::Maximize, "Maximize", SC_MAXIMIZE)
    };
    Some(
        h_flex()
            .tab_group()
            .flex_none()
            .h_full()
            .gap(px(4.))
            .pr(px(6.))
            .when(window.is_minimizable(), |this| {
                this.child(button(
                    "window-minimize",
                    WindowControlArea::Min,
                    IconName::Dash,
                    "Minimize",
                    SC_MINIMIZE,
                ))
            })
            .when(window.is_resizable(), |this| {
                this.child(button(
                    "window-maximize",
                    WindowControlArea::Max,
                    max_icon,
                    max_label,
                    max_command,
                ))
            })
            .child(button(
                "window-close",
                WindowControlArea::Close,
                IconName::Close,
                "Close",
                SC_CLOSE,
            )),
    )
}

#[cfg(windows)]
fn post_window_command(window: &Window, command: u32) {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
    use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_SYSCOMMAND};

    let Ok(handle) = HasWindowHandle::window_handle(window) else {
        return;
    };
    let RawWindowHandle::Win32(handle) = handle.as_raw() else {
        return;
    };
    // SAFETY: 句柄来自当前存活的窗口，系统命令只携带整数参数。
    let result = unsafe {
        PostMessageW(
            Some(HWND(handle.hwnd.get() as _)),
            WM_SYSCOMMAND,
            WPARAM(command as usize),
            LPARAM(0),
        )
    };
    if let Err(error) = result {
        eprintln!("pi-gpui: could not post window command: {error}");
    }
}

/// Room around the window for the shadow; also where resizing starts.
const SHADOW: Pixels = px(10.);
const ROUNDING: Pixels = px(10.);

/// Radii for content that touches the window's corners. GPUI clips children
/// to rectangles only, so backgrounds there (the title bar, the docks) must
/// round themselves to stay inside the frame's rounded border. Zero where
/// the window manager draws the frame or the corner is tiled.
pub fn content_corners(window: &Window) -> Corners<Pixels> {
    let Decorations::Client { tiling } = window.window_decorations() else {
        return Corners::default();
    };
    // Inside the 1 px border.
    let radius = ROUNDING - px(1.);
    let round = |free: bool| if free { radius } else { px(0.) };
    Corners {
        top_left: round(!(tiling.top || tiling.left)),
        top_right: round(!(tiling.top || tiling.right)),
        bottom_left: round(!(tiling.bottom || tiling.left)),
        bottom_right: round(!(tiling.bottom || tiling.right)),
    }
}

pub fn decorate(content: impl IntoElement, window: &mut Window, cx: &mut App) -> AnyElement {
    let Decorations::Client { tiling } = window.window_decorations() else {
        if !cfg!(target_os = "macos") {
            window.set_client_inset(px(0.));
        }
        return content.into_any_element();
    };
    window.set_client_inset(SHADOW);
    let border = cx.theme().colors.border;

    div()
        .id("window-frame")
        .size_full()
        .bg(gpui::transparent_black())
        .when(!tiling.top, |this| this.pt(SHADOW))
        .when(!tiling.bottom, |this| this.pb(SHADOW))
        .when(!tiling.left, |this| this.pl(SHADOW))
        .when(!tiling.right, |this| this.pr(SHADOW))
        // Only moves over the shadow margin get here (the content stops
        // them); repaint so the canvas below picks the resize cursor.
        .on_mouse_move(|_, window, _| window.refresh())
        .on_mouse_down(MouseButton::Left, move |event, window, _| {
            let size = window.window_bounds().get_bounds().size;
            if let Some(edge) = resize_edge(event.position, size, tiling) {
                window.start_window_resize(edge);
            }
        })
        .child(
            div()
                .size_full()
                .overflow_hidden()
                .cursor(CursorStyle::Arrow)
                .border_color(border)
                .when(!tiling.top, |this| this.border_t_1())
                .when(!tiling.bottom, |this| this.border_b_1())
                .when(!tiling.left, |this| this.border_l_1())
                .when(!tiling.right, |this| this.border_r_1())
                .when(!(tiling.top || tiling.left), |this| {
                    this.rounded_tl(ROUNDING)
                })
                .when(!(tiling.top || tiling.right), |this| {
                    this.rounded_tr(ROUNDING)
                })
                .when(!(tiling.bottom || tiling.left), |this| {
                    this.rounded_bl(ROUNDING)
                })
                .when(!(tiling.bottom || tiling.right), |this| {
                    this.rounded_br(ROUNDING)
                })
                .when(!tiling.is_tiled(), |this| {
                    this.shadow(vec![BoxShadow {
                        inset: false,
                        color: Hsla {
                            h: 0.,
                            s: 0.,
                            l: 0.,
                            a: 0.35,
                        },
                        offset: point(px(0.), px(1.)),
                        blur_radius: SHADOW / 2.,
                        spread_radius: px(0.),
                    }])
                })
                // Moves over the content must not reach the frame's edge logic.
                .on_mouse_move(|_, _, cx| cx.stop_propagation())
                .child(content),
        )
        .child(
            canvas(
                |_, window, _| {
                    let size = window.window_bounds().get_bounds().size;
                    window
                        .insert_hitbox(Bounds::new(Point::default(), size), HitboxBehavior::Normal)
                },
                move |_, hitbox, window, _| {
                    let size = window.window_bounds().get_bounds().size;
                    let Some(edge) = resize_edge(window.mouse_position(), size, tiling) else {
                        return;
                    };
                    window.set_cursor_style(
                        match edge {
                            ResizeEdge::Top | ResizeEdge::Bottom => CursorStyle::ResizeUpDown,
                            ResizeEdge::Left | ResizeEdge::Right => CursorStyle::ResizeLeftRight,
                            ResizeEdge::TopLeft | ResizeEdge::BottomRight => {
                                CursorStyle::ResizeUpLeftDownRight
                            }
                            ResizeEdge::TopRight | ResizeEdge::BottomLeft => {
                                CursorStyle::ResizeUpRightDownLeft
                            }
                        },
                        &hitbox,
                    );
                },
            )
            .size_full()
            .absolute(),
        )
        .into_any_element()
}

/// Which edge a press at `position` resizes, if it is in the shadow margin.
fn resize_edge(
    position: Point<Pixels>,
    window: Size<Pixels>,
    tiling: Tiling,
) -> Option<ResizeEdge> {
    let inner = Bounds::new(Point::default(), window).inset(SHADOW * 1.5);
    if inner.contains(&position) {
        return None;
    }
    let corner = size(SHADOW * 1.5, SHADOW * 1.5);
    let corners = [
        (
            Point::default(),
            ResizeEdge::TopLeft,
            !tiling.top && !tiling.left,
        ),
        (
            point(window.width - corner.width, px(0.)),
            ResizeEdge::TopRight,
            !tiling.top && !tiling.right,
        ),
        (
            point(px(0.), window.height - corner.height),
            ResizeEdge::BottomLeft,
            !tiling.bottom && !tiling.left,
        ),
        (
            point(window.width - corner.width, window.height - corner.height),
            ResizeEdge::BottomRight,
            !tiling.bottom && !tiling.right,
        ),
    ];
    for (origin, edge, free) in corners {
        if free && Bounds::new(origin, corner).contains(&position) {
            return Some(edge);
        }
    }
    if position.y < SHADOW && !tiling.top {
        Some(ResizeEdge::Top)
    } else if position.y > window.height - SHADOW && !tiling.bottom {
        Some(ResizeEdge::Bottom)
    } else if position.x < SHADOW && !tiling.left {
        Some(ResizeEdge::Left)
    } else if position.x > window.width - SHADOW && !tiling.right {
        Some(ResizeEdge::Right)
    } else {
        None
    }
}
