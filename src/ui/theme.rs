use gpui::{App, Global, Hsla, rgba};

/// Semantic color tokens, named after Zed's theme schema so components read
/// the same way as Zed's `ui` crate.
#[derive(Clone)]
pub struct ThemeColors {
    pub title_bar_background: Hsla,
    pub title_bar_inactive_background: Hsla,
    pub panel_background: Hsla,
    pub editor_background: Hsla,
    pub elevated_surface_background: Hsla,
    pub element_background: Hsla,
    pub element_hover: Hsla,
    pub element_active: Hsla,
    pub element_selected: Hsla,
    pub ghost_element_hover: Hsla,
    pub ghost_element_active: Hsla,
    pub ghost_element_selected: Hsla,
    pub border: Hsla,
    pub border_variant: Hsla,
    pub border_focused: Hsla,
    pub text: Hsla,
    pub text_muted: Hsla,
    pub text_placeholder: Hsla,
    pub text_disabled: Hsla,
    pub text_accent: Hsla,
    pub icon: Hsla,
    pub icon_muted: Hsla,
    pub icon_disabled: Hsla,
    pub icon_accent: Hsla,
    pub error: Hsla,
    pub error_background: Hsla,
    pub warning: Hsla,
    pub warning_background: Hsla,
    pub success: Hsla,
    pub success_background: Hsla,
    pub info: Hsla,
    pub info_background: Hsla,
    /// Behind fenced code in answers.
    pub code_block_background: Hsla,
    pub syntax: SyntaxColors,
}

/// Code colors, one per scope group (see `highlight.rs`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SyntaxColors {
    pub comment: Hsla,
    pub keyword: Hsla,
    pub string: Hsla,
    pub number: Hsla,
    pub function: Hsla,
    pub r#type: Hsla,
    pub constant: Hsla,
    pub operator: Hsla,
    /// `None` keeps the text color.
    pub punctuation: Option<Hsla>,
    pub property: Hsla,
    pub tag: Hsla,
    pub attribute: Hsla,
}

pub struct Theme {
    pub colors: ThemeColors,
}

impl Global for Theme {}

impl Theme {
    pub fn is_dark(&self) -> bool {
        self.colors.editor_background.l < 0.5
    }

    // 固定导入后的实际配色，避免颜色派生规则变化。来源：https://github.com/dracula/zed
    pub fn dracula(dark: bool) -> Self {
        let colors = if dark {
            ThemeColors {
                title_bar_background: hex(0x141119bb),
                title_bar_inactive_background: hex(0x0a080cbb),
                panel_background: hex(0x16121bff),
                editor_background: hex(0x282a36ff),
                elevated_surface_background: hex(0x1e1925ff),
                element_background: hex(0x1c1d26ff),
                element_hover: hex(0x3c324bff),
                element_active: hex(0x796595ff),
                element_selected: hex(0x282232ff),
                ghost_element_hover: hex(0xc9a8f935),
                ghost_element_active: hex(0xc9a8f950),
                ghost_element_selected: hex(0xc9a8f925),
                border: hex(0x3c324bcc),
                border_variant: hex(0xc9a8f933),
                border_focused: hex(0xc9a8f977),
                text: hex(0xf8f8f2ff),
                text_muted: hex(0xa186c7ff),
                text_placeholder: hex(0xc9a8f980),
                text_disabled: hex(0xc9a8f950),
                text_accent: hex(0xc9a8f9ff),
                icon: hex(0xf8f8f2ff),
                icon_muted: hex(0xf8f8f270),
                icon_disabled: hex(0xf8f8f240),
                icon_accent: hex(0xc9a8f9ff),
                error: hex(0xe67373ff),
                error_background: hex(0x242631ee),
                warning: hex(0xe6e373ff),
                warning_background: hex(0x242631ee),
                success: hex(0x79e96dff),
                success_background: hex(0x242631ee),
                info: hex(0x73ece5ff),
                info_background: hex(0x242631ee),
                code_block_background: hex(0x000000ff).opacity(0.25),
                syntax: SyntaxColors {
                    comment: hex(0x6272a4ff),
                    keyword: hex(0xff79c6ff),
                    string: hex(0xf1fa8cff),
                    number: hex(0xbd93f9ff),
                    function: hex(0x50fa7bff),
                    r#type: hex(0x8be9fdff),
                    constant: hex(0xbd93f9ff),
                    operator: hex(0xff79c6ff),
                    property: hex(0x8be9fdff),
                    tag: hex(0xff79c6ff),
                    attribute: hex(0x8be9fdff),
                    punctuation: Some(hex(0xff79c6ff)),
                },
            }
        } else {
            ThemeColors {
                title_bar_background: hex(0xfefefeff),
                title_bar_inactive_background: hex(0xf0f0f0ff),
                panel_background: hex(0xedeff4ff),
                editor_background: hex(0xf5f5f5ff),
                elevated_surface_background: hex(0xf6f6f6ff),
                element_background: hex(0xdce0e8ff),
                element_hover: hex(0xccd0daff),
                element_active: hex(0xd6d9e1ff),
                element_selected: hex(0xcfcfdeff),
                ghost_element_hover: hex(0xccd0daff),
                ghost_element_active: hex(0xd6d9e1ff),
                ghost_element_selected: hex(0xdce0e8ff),
                border: hex(0xd1c9efff),
                border_variant: hex(0xe4e4e4ff),
                border_focused: hex(0xf5f5f577),
                text: hex(0x1f1f1fff),
                text_muted: hex(0x353535ff),
                text_placeholder: hex(0x1f1f1f80),
                text_disabled: hex(0x1f1f1f70),
                text_accent: hex(0x836ed4ff),
                icon: hex(0x1f1f1fff),
                icon_muted: hex(0x1f1f1f70),
                icon_disabled: hex(0x1f1f1f40),
                icon_accent: hex(0x836ed4ff),
                error: hex(0xb83233ff),
                error_background: hex(0xddddddee),
                warning: hex(0xccb517ff),
                warning_background: hex(0xf0f0f0ee),
                success: hex(0x1b7108ff),
                success_background: hex(0xf0f0f0ee),
                info: hex(0x0067a1ff),
                info_background: hex(0xf0f0f0ee),
                code_block_background: hex(0xeeeeeeff),
                syntax: SyntaxColors {
                    comment: hex(0x635d97ff),
                    keyword: hex(0xa3144dff),
                    string: hex(0x846e15ff),
                    number: hex(0x644ac9ff),
                    function: hex(0x14710aff),
                    r#type: hex(0x036a96ff),
                    constant: hex(0x644ac9ff),
                    operator: hex(0xa3144dff),
                    property: hex(0x036a96ff),
                    tag: hex(0xa3144dff),
                    attribute: hex(0x036a96ff),
                    punctuation: Some(hex(0xa3144dff)),
                },
            }
        };
        Self { colors }
    }
}

pub trait ActiveTheme {
    fn theme(&self) -> &Theme;
}

impl ActiveTheme for App {
    fn theme(&self) -> &Theme {
        self.global::<Theme>()
    }
}

/// Semantic text and icon colors, as in Zed's `ui::Color`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Color {
    #[default]
    Default,
    Muted,
    Placeholder,
    Disabled,
    Accent,
    Error,
    Warning,
    Success,
}

impl Color {
    pub fn text(self, cx: &App) -> Hsla {
        let colors = &cx.theme().colors;
        match self {
            Color::Default => colors.text,
            Color::Muted => colors.text_muted,
            Color::Placeholder => colors.text_placeholder,
            Color::Disabled => colors.text_disabled,
            Color::Accent => colors.text_accent,
            Color::Error => colors.error,
            Color::Warning => colors.warning,
            Color::Success => colors.success,
        }
    }

    pub fn icon(self, cx: &App) -> Hsla {
        let colors = &cx.theme().colors;
        match self {
            Color::Default => colors.icon,
            Color::Muted => colors.icon_muted,
            Color::Placeholder | Color::Disabled => colors.icon_disabled,
            Color::Accent => colors.icon_accent,
            _ => self.text(cx),
        }
    }
}

fn hex(value: u32) -> Hsla {
    rgba(value).into()
}
