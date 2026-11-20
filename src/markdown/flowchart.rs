use std::collections::HashSet;
use std::sync::{Arc, LazyLock};

use gpui::{
    AnyElement, App, Asset, Global, RenderImage, SharedString, SvgSize, Window, div, img, px, size,
};
use mermaid_rs_renderer::{RenderOptions, render_strict};
use regex::Regex;

use gpui::prelude::*;

pub(super) const MAX_SOURCE_BYTES: usize = 4096;
const MAX_CACHED: usize = 16;

#[derive(IntoElement)]
pub(super) struct Diagram {
    key: SharedString,
    source: SharedString,
    code: AnyElement,
}

impl Diagram {
    pub(super) fn new(key: SharedString, source: SharedString, code: AnyElement) -> Self {
        Self { key, source, code }
    }
}

impl RenderOnce for Diagram {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        if !cx.has_global::<Diagrams>() {
            cx.set_global(Diagrams::default());
        }
        let (admitted, first) = cx.update_global::<Diagrams, _>(|diagrams, _| {
            let first = diagrams.seen.is_empty();
            (diagrams.admit(self.source.clone()), first)
        });
        if first {
            window.on_next_frame(|window, cx| {
                let (expired, retry) =
                    cx.update_global::<Diagrams, _>(|diagrams, _| diagrams.finish_frame());
                for source in expired {
                    cx.remove_asset::<Flowchart>(&source);
                }
                if retry {
                    window.refresh();
                }
            });
        }
        let image = admitted
            .then(|| window.use_asset::<Flowchart>(&self.source, cx))
            .flatten()
            .flatten();
        let Some(image) = image else {
            return self.code;
        };
        div()
            .id(self.key)
            .overflow_x_scroll()
            .my(px(8.))
            .child(img(image).max_w(px(760.)))
            .into_any_element()
    }
}

#[derive(Default)]
struct Diagrams {
    cached: HashSet<SharedString>,
    seen: HashSet<SharedString>,
    blocked: bool,
}

impl Global for Diagrams {}

impl Diagrams {
    fn admit(&mut self, source: SharedString) -> bool {
        self.seen.insert(source.clone());
        if self.cached.contains(&source) {
            return true;
        }
        if self.cached.len() == MAX_CACHED {
            self.blocked = true;
            return false;
        }
        self.cached.insert(source);
        true
    }

    fn finish_frame(&mut self) -> (Vec<SharedString>, bool) {
        let mut expired = Vec::new();
        if std::mem::take(&mut self.blocked) {
            // 当前帧仍可见的图不能被逐个驱逐，否则每个加载结果都会触发下一轮加载。
            self.cached.retain(|source| {
                if self.seen.contains(source) {
                    return true;
                }
                expired.push(source.clone());
                false
            });
        }
        self.seen.clear();
        let retry = !expired.is_empty();
        (expired, retry)
    }
}

struct Flowchart;

impl Asset for Flowchart {
    type Source = SharedString;
    type Output = Option<Arc<RenderImage>>;

    fn load(
        source: Self::Source,
        cx: &mut App,
    ) -> impl Future<Output = Self::Output> + Send + 'static {
        let renderer = cx.svg_renderer();
        async move {
            let svg = diagram_svg(&source)?;
            let dimensions = svg.split_once("viewBox=\"")?.1.split('"').next()?;
            let mut dimensions = dimensions.split_whitespace().skip(2);
            let width: f32 = dimensions.next()?.parse().ok()?;
            let height: f32 = dimensions.next()?.parse().ok()?;
            let scale = (1200. / width.max(height)).min(2.0);
            let tree = renderer.parse_svg(svg.as_bytes()).ok()?;
            renderer
                .render_parsed(
                    &tree,
                    SvgSize::ExactSize(size(
                        ((width * scale).round() as i32).into(),
                        ((height * scale).round() as i32).into(),
                    )),
                )
                .ok()
        }
    }
}

fn diagram_svg(source: &str) -> Option<String> {
    if source.len() > MAX_SOURCE_BYTES {
        return None;
    }
    const LABEL: &str = r#"[^\[\](){}<>|"&;]{1,80}"#;
    static STATEMENT: LazyLock<Regex> = LazyLock::new(|| {
        let id = r"[A-Za-z_][A-Za-z_0-9]*";
        let label = format!(r#"(?:"{LABEL}"|{LABEL})"#);
        let node = format!(r"{id}(?:\[{label}\]|\({label}\)|\{{{label}\}})?");
        Regex::new(&format!(r"^(?:{node}(?:\s*(?:-->|-\.->)\s*(?:\|{label}\|\s*)?{node})?|(?P<group>subgraph)\s+{id}(?:\s*\[{label}\])?)$")).expect("valid flowchart subset regex")
    });
    let mut lines = source
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with("%%"));
    let mut header = lines.next()?.split_whitespace();
    if !matches!(header.next(), Some("graph" | "flowchart"))
        || !matches!(header.next(), Some("LR" | "RL" | "TD" | "TB" | "BT"))
        || header.next().is_some()
    {
        return None;
    }
    let mut in_group = false;
    let mut statements = 0;
    for line in lines {
        // 解析器可能跳过未知语句，先逐行验证才能避免只画出部分流程。
        if line == "end" {
            if !in_group {
                return None;
            }
            in_group = false;
        } else {
            let captures = STATEMENT.captures(line)?;
            if captures.name("group").is_some() {
                if in_group {
                    return None;
                }
                in_group = true;
            } else if line.replace("-->", "").contains("--")
                || matches!(
                    line.split([' ', '[', '(', '{']).next(),
                    Some(
                        "subgraph"
                            | "direction"
                            | "style"
                            | "class"
                            | "classDef"
                            | "linkStyle"
                            | "click"
                    )
                )
            {
                return None;
            }
        }
        statements += 1;
    }
    if in_group || !(1..=40).contains(&statements) || source.contains("%%{") {
        return None;
    }
    render_strict(source, RenderOptions::mermaid_default()).ok()
}

#[cfg(test)]
mod tests {
    use super::{Diagrams, MAX_CACHED, diagram_svg};

    #[test]
    fn more_visible_diagrams_than_cache_slots_do_not_reload_each_other() {
        let mut cache = Diagrams::default();
        for _ in 0..2 {
            for index in 0..=MAX_CACHED {
                assert_eq!(cache.admit(index.to_string().into()), index < MAX_CACHED);
            }
            assert!(!cache.finish_frame().1);
        }
        assert!(!cache.admit("another".into()));
        assert!(cache.finish_frame().1);
        assert!(cache.admit("another".into()));
    }

    #[test]
    fn chinese_branches_and_groups_are_rendered_in_both_directions() {
        for header in ["graph LR", "flowchart TD"] {
            let svg = diagram_svg(&format!("{header}\nsubgraph S [校验]\nA[收到消息] --> B{{完整吗}}\nend\nB -->|是| C[渲染]\nB -.->|否| D[等待]"))
                .expect("representative flowchart should render");
            for label in ["校验", "收到消息", "完整吗", "是", "否", "渲染", "等待"] {
                assert!(svg.contains(label), "missing {label} in {header}");
            }
            assert!(
                svg.matches("<g transform=\"translate(").count() >= 3,
                "{header}"
            );
        }
    }

    #[test]
    fn a_single_unsupported_or_invalid_line_rejects_the_whole_diagram() {
        for line in [
            "B -->",
            "C & D --> E",
            "click A \"https://example.com\"",
            "subgraph S",
            "style A fill:red",
            r#"C["Run pi --version"]"#,
        ] {
            assert!(
                diagram_svg(&format!("graph LR\nA --> B\n{line}")).is_none(),
                "{line}"
            );
        }
    }
}
