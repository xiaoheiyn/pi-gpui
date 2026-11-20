use std::collections::VecDeque;
use std::sync::{Arc, LazyLock};

use gpui::{
    AnyElement, App, Asset, Global, RenderImage, SharedString, SvgSize, Window, div, img, px, size,
};
use mermaid_rs_renderer::{RenderOptions, render_strict};
use regex::Regex;

use gpui::prelude::*;

#[derive(IntoElement)]
pub(super) struct Diagram {
    pub key: SharedString,
    pub source: SharedString,
    pub code: AnyElement,
}

impl RenderOnce for Diagram {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        if !cx.has_global::<Diagrams>() {
            cx.set_global(Diagrams::default());
        }
        let expired = cx.update_global::<Diagrams, _>(|diagrams, _| {
            let entries = &mut diagrams.0;
            entries.retain(|entry| entry != &self.source);
            entries.push_back(self.source.clone());
            // 限定图像缓存大小，避免长会话不断保留位图。
            (entries.len() > 16).then(|| entries.pop_front()).flatten()
        });
        if let Some(expired) = expired {
            cx.remove_asset::<Flowchart>(&expired);
        }
        let Some(Some(image)) = window.use_asset::<Flowchart>(&self.source, cx) else {
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
struct Diagrams(VecDeque<SharedString>);

impl Global for Diagrams {}

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
    if source.len() > 4096 {
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
            } else if matches!(
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
            ) {
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
    use super::diagram_svg;

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
        ] {
            assert!(
                diagram_svg(&format!("graph LR\nA --> B\n{line}")).is_none(),
                "{line}"
            );
        }
    }
}
