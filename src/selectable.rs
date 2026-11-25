//! Read-only text that can be selected with the mouse and copied: messages,
//! thoughts and tool output in the transcript. Plain GPUI text cannot be
//! selected, so this element shapes and paints its text itself.
//!
//! 选区属于逻辑文档，因为虚拟列表会在滚动时重新创建文本元素。

use std::cell::{OnceCell, RefCell};
use std::collections::BTreeMap;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use gpui::{
    App, AvailableSpace, Bounds, Context, CursorStyle, DispatchPhase, ElementId, Entity,
    FocusHandle, FontStyle, FontWeight, GlobalElementId, Hitbox, HitboxBehavior, Hsla, LayoutId,
    MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, PaintQuad, Pixels, Point,
    SharedString, StrikethroughStyle, Style, TextAlign, TextRun, UnderlineStyle, Window,
    WrappedLine, fill, point, prelude::*, px, relative, size,
};

use crate::text_input::{TextLayout, paragraph_range_at, word_range_at};
use crate::ui::{ActiveTheme as _, Color, LabelSize};

/// Inline formatting for a byte range of a [`SelectableText`].
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SpanStyle {
    pub bold: bool,
    pub italic: bool,
    pub code: bool,
    pub strike: bool,
    /// Syntax highlighting color.
    pub color: Option<Hsla>,
    /// Opened on a plain click (one that selects nothing).
    pub link: Option<SharedString>,
}

pub struct TextSelection {
    // 仍由起点元素处理拖动和链接点击，避免多个片段同时改写选区。
    origin: Option<SharedString>,
    document: Option<SharedString>,
    text: SharedString,
    anchor: usize,
    head: usize,
    dragging: bool,
    /// Receives keyboard focus on selection, so Copy reaches the owning view.
    copy_focus: FocusHandle,
    /// Relative file links resolve against this folder (the session's project).
    link_base: Option<PathBuf>,
}

impl TextSelection {
    pub fn new(copy_focus: FocusHandle) -> Self {
        Self {
            origin: None,
            document: None,
            text: SharedString::default(),
            anchor: 0,
            head: 0,
            dragging: false,
            copy_focus,
            link_base: None,
        }
    }

    pub fn with_link_base(mut self, base: PathBuf) -> Self {
        self.link_base = Some(base);
        self
    }

    pub fn selected_text(&self) -> Option<String> {
        let range = self.range();
        if self.origin.is_none() || range.is_empty() {
            return None;
        }
        self.text.get(range).map(str::to_owned)
    }

    pub fn clear(&mut self, cx: &mut Context<Self>) {
        if self.origin.take().is_some() {
            self.document = None;
            self.text = SharedString::default();
            self.dragging = false;
            cx.notify();
        }
    }

    fn range(&self) -> Range<usize> {
        self.anchor.min(self.head)..self.anchor.max(self.head)
    }

    fn range_in(&self, key: &SharedString) -> Option<Range<usize>> {
        (self.document.as_ref() == Some(key)).then(|| self.range())
    }
}

pub(crate) struct TextDocument {
    key: SharedString,
    text: OnceCell<SharedString>,
    // 坐标只属于这一轮渲染，不能随选区跨帧保存。
    fragments: RefCell<BTreeMap<usize, TextFragment>>,
}

struct TextFragment {
    len: usize,
    bounds: Bounds<Pixels>,
    layout: Rc<TextLayout>,
}

impl TextDocument {
    pub(crate) fn new(key: &str) -> Self {
        Self {
            key: key.to_owned().into(),
            text: OnceCell::new(),
            fragments: RefCell::new(BTreeMap::new()),
        }
    }

    pub(crate) fn finish(&self, text: String) {
        self.text
            .set(text.into())
            .expect("selection document is built once per render");
    }

    fn offset_for_position(&self, position: Point<Pixels>) -> Option<usize> {
        let fragments = self.fragments.borrow();
        let (&start, fragment) = fragments.iter().min_by_key(|(_, fragment)| {
            let bounds = fragment.bounds;
            let vertical = (bounds.top() - position.y).max(position.y - bounds.bottom());
            let horizontal = (bounds.left() - position.x).max(position.x - bounds.right());
            (vertical.max(px(0.)), horizontal.max(px(0.)))
        })?;
        let offset = if position.y < fragment.bounds.top() {
            0
        } else if position.y >= fragment.bounds.bottom() {
            fragment.len
        } else {
            fragment
                .layout
                .offset_for_position(position - fragment.bounds.origin)
        };
        Some(start + offset)
    }
}

pub struct SelectableText {
    key: SharedString,
    text: SharedString,
    document: Option<(Rc<TextDocument>, usize)>,
    selection: Entity<TextSelection>,
    color: Option<Color>,
    size: Option<LabelSize>,
    /// Font size and line height, overriding `size`.
    text_size: Option<(Pixels, Pixels)>,
    weight: Option<FontWeight>,
    italic: bool,
    monospace: bool,
    font_family: Option<SharedString>,
    /// Lay out on one line per hard line break and report the natural width.
    no_wrap: bool,
    spans: Rc<[(Range<usize>, SpanStyle)]>,
}

impl SelectableText {
    /// `key` identifies the block across frames (e.g. `"12:text"`).
    pub fn new(
        key: impl Into<SharedString>,
        text: impl Into<SharedString>,
        selection: &Entity<TextSelection>,
    ) -> Self {
        Self {
            key: key.into(),
            text: text.into(),
            document: None,
            selection: selection.clone(),
            color: None,
            size: None,
            text_size: None,
            weight: None,
            italic: false,
            monospace: false,
            font_family: None,
            no_wrap: false,
            spans: Rc::from([]),
        }
    }

    pub(crate) fn in_document(mut self, document: Rc<TextDocument>, fragment: usize) -> Self {
        self.document = Some((document, fragment));
        self
    }

    pub fn font_family(mut self, family: impl Into<SharedString>) -> Self {
        self.font_family = Some(family.into());
        self
    }

    pub fn no_wrap(mut self) -> Self {
        self.no_wrap = true;
        self
    }

    /// Sorted, non-overlapping styled ranges.
    pub fn spans(mut self, spans: Vec<(Range<usize>, SpanStyle)>) -> Self {
        self.spans = spans.into();
        self
    }

    pub fn text_size(mut self, font_size: Pixels, line_height: Pixels) -> Self {
        self.text_size = Some((font_size, line_height));
        self
    }

    pub fn weight(mut self, weight: FontWeight) -> Self {
        self.weight = Some(weight);
        self
    }

    pub fn color(mut self, color: Color) -> Self {
        self.color = Some(color);
        self
    }

    pub fn size(mut self, size: LabelSize) -> Self {
        self.size = Some(size);
        self
    }

    pub fn italic(mut self) -> Self {
        self.italic = true;
        self
    }

    pub fn monospace(mut self) -> Self {
        self.monospace = true;
        self
    }

    /// Runs, font size and line height derived from the inherited text style.
    fn text_params(&self, window: &Window, cx: &App) -> (Vec<TextRun>, Pixels, Pixels) {
        let mut style = window.text_style();
        let code_font = crate::settings::Settings::get(cx)
            .buffer_font_family
            .clone();
        if self.monospace {
            style.font_family = code_font.clone();
        }
        if let Some(family) = &self.font_family {
            style.font_family = family.clone();
        }
        if self.italic {
            style.font_style = FontStyle::Italic;
        }
        if let Some(weight) = self.weight {
            style.font_weight = weight;
        }
        let (font_size, line_height) = match (self.text_size, self.size) {
            (Some(text_size), _) => text_size,
            (None, Some(size)) => (size.pixels(), size.line_height()),
            (None, None) => (
                style.font_size.to_pixels(window.rem_size()),
                window.line_height(),
            ),
        };
        let base = TextRun {
            len: 0,
            font: style.font(),
            color: self.color.map_or(style.color, |color| color.text(cx)),
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let colors = &cx.theme().colors;
        let mut runs = Vec::with_capacity(self.spans.len() * 2 + 1);
        let mut position = 0;
        let len = self.text.len();
        for (range, span) in self.spans.iter() {
            let range = range.start.clamp(position, len)..range.end.clamp(position, len);
            if range.start > position {
                runs.push(TextRun {
                    len: range.start - position,
                    ..base.clone()
                });
            }
            if range.is_empty() {
                continue;
            }
            let mut run = TextRun {
                len: range.len(),
                ..base.clone()
            };
            // Inline styles follow Zed's agent markdown style.
            if let Some(color) = span.color {
                run.color = color;
            }
            if span.bold {
                run.font.weight = FontWeight::BOLD;
                run.color = colors.text;
            }
            if span.italic {
                run.font.style = FontStyle::Italic;
            }
            if span.code {
                run.font.family = code_font.clone();
                run.background_color = Some(colors.text.opacity(0.08));
            }
            if span.strike {
                run.strikethrough = Some(StrikethroughStyle {
                    thickness: px(1.),
                    color: Some(run.color),
                });
            }
            if span.link.is_some() {
                run.color = colors.text_accent;
                run.background_color = Some(colors.text.opacity(0.025));
                run.underline = Some(UnderlineStyle {
                    thickness: px(1.),
                    color: Some(colors.text_accent.opacity(0.5)),
                    wavy: false,
                });
            }
            runs.push(run);
            position = range.end;
        }
        if position < len || runs.is_empty() {
            runs.push(TextRun {
                len: len - position,
                ..base
            });
        }
        (runs, font_size, line_height)
    }

    pub(crate) fn unwrapped_width(&self, window: &Window, cx: &App) -> Pixels {
        let (runs, font_size, line_height) = self.text_params(window, cx);
        window
            .text_system()
            .shape_text(self.text.clone(), font_size, &runs, None, None)
            .map_or(px(0.), |lines| widest_line(&lines, line_height))
    }

    fn shape(&self, width: Option<Pixels>, window: &mut Window, cx: &App) -> Option<TextLayout> {
        let (runs, font_size, line_height) = self.text_params(window, cx);
        let lines = window
            .text_system()
            .shape_text(self.text.clone(), font_size, &runs, width, None)
            .ok()?;
        Some(TextLayout::new(lines.into_vec(), line_height))
    }
}

fn widest_line(lines: &[WrappedLine], line_height: Pixels) -> Pixels {
    lines
        .iter()
        .map(|line| line.size(line_height).width)
        .fold(px(0.), Pixels::max)
}

pub struct PrepaintState {
    layout: Option<Rc<TextLayout>>,
    hitbox: Hitbox,
    selections: Vec<PaintQuad>,
}

impl Element for SelectableText {
    type RequestLayoutState = ();
    type PrepaintState = PrepaintState;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let (runs, font_size, line_height) = self.text_params(window, cx);
        let text = self.text.clone();
        let no_wrap = self.no_wrap;
        let mut style = Style::default();
        if no_wrap {
            // Natural width; a scrolling parent shows what does not fit.
            style.flex_shrink = 0.;
        } else {
            style.size.width = relative(1.).into();
        }
        let layout_id =
            window.request_measured_layout(style, move |known, available, window, _cx| {
                let width = if no_wrap {
                    None
                } else {
                    known.width.or(match available.width {
                        AvailableSpace::Definite(width) => Some(width),
                        _ => None,
                    })
                };
                let lines = window
                    .text_system()
                    .shape_text(text.clone(), font_size, &runs, width, None)
                    .unwrap_or_default();
                let height = lines
                    .iter()
                    .map(|line| line.size(line_height).height)
                    .fold(px(0.), |total, height| total + height)
                    .max(line_height);
                let width = width.unwrap_or_else(|| widest_line(&lines, line_height));
                size(width, height)
            });
        (layout_id, ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let hitbox = window.insert_hitbox(bounds, HitboxBehavior::Normal);
        let wrap_width = (!self.no_wrap).then_some(bounds.size.width);
        let layout = self.shape(wrap_width, window, cx).map(Rc::new);
        let mut selections = Vec::new();
        let (document_key, start) = self
            .document
            .as_ref()
            .map_or((&self.key, 0), |(doc, start)| (&doc.key, *start));
        if let (Some(layout), Some(range)) =
            (&layout, self.selection.read(cx).range_in(document_key))
        {
            let local = |offset: usize| offset.saturating_sub(start).min(self.text.len());
            let range = local(range.start)..local(range.end);
            if !range.is_empty() {
                let start = layout.position_for_offset(range.start);
                let end = layout.position_for_offset(range.end);
                let color = cx.theme().colors.info.opacity(0.3);
                let line_height = layout.line_height;
                let mut row_top = start.y;
                while row_top <= end.y {
                    let left = if row_top == start.y { start.x } else { px(0.) };
                    let right = if row_top == end.y {
                        end.x
                    } else {
                        bounds.size.width
                    };
                    if right > left {
                        selections.push(fill(
                            Bounds::from_corners(
                                bounds.origin + point(left, row_top),
                                bounds.origin + point(right, row_top + line_height),
                            ),
                            color,
                        ));
                    }
                    row_top += line_height;
                }
            }
        }
        PrepaintState {
            layout,
            hitbox,
            selections,
        }
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        window.set_cursor_style(CursorStyle::IBeam, &prepaint.hitbox);
        let Some(layout) = prepaint.layout.clone() else {
            return;
        };
        for (line, top) in layout.lines.iter().zip(&layout.line_tops) {
            if !self.spans.is_empty() {
                line.paint_background(
                    bounds.origin + point(px(0.), *top),
                    layout.line_height,
                    TextAlign::Left,
                    None,
                    window,
                    cx,
                )
                .ok();
            }
            if let Err(error) = line.paint(
                bounds.origin + point(px(0.), *top),
                layout.line_height,
                TextAlign::Left,
                None,
                window,
                cx,
            ) {
                eprintln!("pi-gpui: failed to paint text: {error}");
            }
        }

        // Over the text, so code backgrounds do not hide it.
        for selection in prepaint.selections.drain(..) {
            window.paint_quad(selection);
        }

        let origin = bounds.origin;
        let hitbox = prepaint.hitbox.clone();
        let selection = self.selection.clone();
        let key = self.key.clone();
        let text = self.text.clone();
        let document = self.document.clone();
        if let Some((document, start)) = &document
            && !text.is_empty()
        {
            document.fragments.borrow_mut().insert(
                *start,
                TextFragment {
                    len: text.len(),
                    bounds,
                    layout: layout.clone(),
                },
            );
        }
        window.on_mouse_event({
            let layout = layout.clone();
            let selection = selection.clone();
            let key = key.clone();
            let document = document.clone();
            move |event: &MouseDownEvent, phase, window, cx| {
                if phase != DispatchPhase::Bubble || event.button != MouseButton::Left {
                    return;
                }
                if hitbox.is_hovered(window) {
                    let offset = layout.offset_for_position(event.position - origin);
                    let range = match event.click_count {
                        2 => word_range_at(&text, offset),
                        3.. => paragraph_range_at(&text, offset),
                        _ => offset..offset,
                    };
                    let focus = selection.update(cx, |selection, cx| {
                        let (document_key, start, source) = document
                            .as_ref()
                            .and_then(|(doc, start)| Some((&doc.key, *start, doc.text.get()?)))
                            .unwrap_or((&key, 0, &text));
                        selection.origin = Some(key.clone());
                        selection.document = Some(document_key.clone());
                        selection.text = source.clone();
                        selection.anchor = start + range.start;
                        selection.head = start + range.end;
                        selection.dragging = event.click_count == 1;
                        cx.notify();
                        selection.copy_focus.clone()
                    });
                    window.focus(&focus, cx);
                } else {
                    // A click elsewhere ends this block's selection. If the click
                    // started a selection in another block, that block owns it now.
                    selection.update(cx, |selection, cx| {
                        if selection.origin.as_ref() == Some(&key) {
                            selection.clear(cx);
                        }
                    });
                }
            }
        });
        window.on_mouse_event({
            let layout = layout.clone();
            let selection = selection.clone();
            let key = key.clone();
            move |event: &MouseMoveEvent, phase, _window, cx| {
                if phase != DispatchPhase::Bubble || !event.dragging() {
                    return;
                }
                let current = selection.read(cx);
                if !current.dragging || current.origin.as_ref() != Some(&key) {
                    return;
                }
                let (offset, source) = match &document {
                    Some((document, start)) => (
                        document
                            .offset_for_position(event.position)
                            .unwrap_or_else(|| {
                                start + layout.offset_for_position(event.position - origin)
                            }),
                        document.text.get(),
                    ),
                    None => (layout.offset_for_position(event.position - origin), None),
                };
                let source = source.unwrap_or(&current.text);
                if source.is_char_boundary(current.anchor) && source.is_char_boundary(offset) {
                    let source = source.clone();
                    selection.update(cx, |selection, cx| {
                        if selection.head != offset || selection.text != source {
                            selection.head = offset;
                            selection.text = source;
                            cx.notify();
                        }
                    });
                }
            }
        });
        let links: Vec<(Range<usize>, SharedString)> = self
            .spans
            .iter()
            .filter_map(|(range, span)| Some((range.clone(), span.link.clone()?)))
            .collect();
        let hitbox = prepaint.hitbox.clone();
        window.on_mouse_event(move |event: &MouseUpEvent, phase, window, cx| {
            if phase != DispatchPhase::Bubble || event.button != MouseButton::Left {
                return;
            }
            let clicked = selection.update(cx, |selection, _| {
                let owned = selection.origin.as_ref() == Some(&key);
                let clicked = owned && selection.dragging && selection.range().is_empty();
                if owned {
                    selection.dragging = false;
                }
                clicked
            });
            // A click that selected nothing follows a link under the pointer.
            if clicked && !links.is_empty() && hitbox.is_hovered(window) {
                let offset = layout.offset_for_position(event.position - origin);
                if let Some((_, url)) = links.iter().find(|(range, _)| range.contains(&offset)) {
                    let base = selection.read(cx).link_base.clone();
                    open_link(url, base.as_deref(), cx);
                }
            }
        });
    }
}

/// Opens a link from an answer. Web and mail links go to the browser. Anything
/// else is a file: `file://` URLs, absolute paths, `~/...` and paths relative
/// to the project, optionally with a `#anchor` or a `:line` suffix. macOS
/// cannot open a bare path as a URL (error -50), so files open with the
/// system's default app.
pub fn open_link(link: &str, base: Option<&Path>, cx: &App) {
    let link = link.trim();
    let scheme = link
        .split_once(':')
        .map(|(scheme, _)| scheme.to_ascii_lowercase());
    match scheme.as_deref() {
        Some("http" | "https" | "mailto") => return cx.open_url(link),
        Some("file") => {}
        // Other schemes (`vscode:`, `zed:`...) but not Windows-like `C:` or `src/a.rs:12`.
        Some(scheme)
            if scheme.len() > 1
                && scheme
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || "+.-".contains(c))
                && !link.contains(' ')
                && link[scheme.len() + 1..].starts_with("//") =>
        {
            return cx.open_url(link);
        }
        _ => {}
    }
    match resolve_file_link(link, base) {
        Some(path) => cx.open_with_system(&path),
        None => eprintln!("pi-gpui: link target not found: {link}"),
    }
}

fn resolve_file_link(link: &str, base: Option<&Path>) -> Option<PathBuf> {
    let raw = link.strip_prefix("file://").unwrap_or(link);
    let raw = raw.split('#').next().unwrap_or(raw);
    let decoded = percent_decode(raw);
    let mut candidates = vec![decoded.clone()];
    // `src/main.rs:12` or `src/main.rs:12:5`.
    let mut trimmed = decoded.as_str();
    for _ in 0..2 {
        match trimmed.rsplit_once(':') {
            Some((head, tail)) if !tail.is_empty() && tail.chars().all(|c| c.is_ascii_digit()) => {
                trimmed = head;
                candidates.push(trimmed.to_string());
            }
            _ => break,
        }
    }
    candidates.into_iter().find_map(|candidate| {
        let path = if let Some(rest) = candidate.strip_prefix("~/") {
            crate::sessions::home_directory()?.join(rest)
        } else if Path::new(&candidate).is_absolute() {
            PathBuf::from(&candidate)
        } else {
            base?.join(&candidate)
        };
        path.exists().then_some(path)
    })
}

/// `%20` and friends; invalid sequences stay as they are.
fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%'
            && let Some(byte) = text
                .get(index + 1..index + 3)
                .and_then(|hex| u8::from_str_radix(hex, 16).ok())
        {
            out.push(byte);
            index += 3;
            continue;
        }
        out.push(bytes[index]);
        index += 1;
    }
    String::from_utf8(out).unwrap_or_else(|_| text.to_string())
}

#[cfg(test)]
mod link_tests {
    use super::*;

    #[test]
    fn file_links_resolve_like_paths() {
        let root = std::env::temp_dir().join(format!("pi-gpui-links-{}", std::process::id()));
        std::fs::create_dir_all(root.join("docs")).expect("dir");
        let file = root.join("docs/my cv.pdf");
        std::fs::write(&file, b"x").expect("file");

        assert_eq!(
            resolve_file_link("docs/my cv.pdf", Some(&root)),
            Some(file.clone())
        );
        assert_eq!(
            resolve_file_link("docs/my%20cv.pdf", Some(&root)),
            Some(file.clone())
        );
        let absolute = format!("file://{}", file.display()).replace(' ', "%20");
        assert_eq!(resolve_file_link(&absolute, None), Some(file.clone()));
        assert_eq!(
            resolve_file_link(&format!("{}:12", file.display()), None),
            Some(file.clone())
        );
        assert_eq!(
            resolve_file_link("docs/my cv.pdf#page=2", Some(&root)),
            Some(file)
        );
        assert_eq!(resolve_file_link("docs/missing.pdf", Some(&root)), None);

        std::fs::remove_dir_all(&root).ok();
    }
}
