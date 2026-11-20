# gpui-fast 对现有桌面功能和构建流程的兼容范围

**建议用 Longbridge 发布的 `gpui-fast =0.1.17` 与 `gpui-fast-platform =0.1.17` 作为迁移候选。** 对应标签 `v0.1.17` 解引用到 `bf313b428e28f0bfd14aaa7bbd25c4372f4052de`，上游记录的 Zed 快照是 `78f5d7446c55f6c1aa9b66e8111f6b3f4625ebb0`。候选有 macOS Metal、Linux Wayland 和 X11 后端，但现有 Rust 1.90 工具链和部分应用调用需要调整。当前可确认到依赖解析和源码接口层，发布兼容仍需编译和真机验收。[标签](https://github.com/longbridge/gpui-fast/releases/tag/v0.1.17) · [UPSTREAM](https://github.com/longbridge/gpui-fast/blob/bf313b428e28f0bfd14aaa7bbd25c4372f4052de/UPSTREAM) · [平台入口](https://github.com/longbridge/gpui-fast/blob/bf313b428e28f0bfd14aaa7bbd25c4372f4052de/crates/gpui_platform/src/gpui_platform.rs)

本仓库基线为 `a8d40e30c4ea6f7091e5e0ee8e819e2c3bbe6da2`（Pi GPUI 0.2.8）。现用 crates.io 的 `gpui =0.2.2` 加 `runtime_shaders`。公开支持范围是 macOS 13+ Apple Silicon，以及 Linux x86_64／arm64、glibc 2.35+、Vulkan、X11／Wayland（beta）。Windows 不在产品支持范围。[依赖](../../../Cargo.toml) · [锁文件](../../../Cargo.lock#L1990) · [安装与平台说明](../../../README.md#install)

## 固定依赖与工具链

以下是临时目录里解析过的依赖配置。Cargo 左侧别名保留应用的 `use gpui::…`，`package` 指向 crates.io 发布名。平台包与核心分开选 feature。仓库源码的包名是 `gpui`、`gpui_platform`，发布脚本会改写，直接把这两个 registry 配置改成 Git URL 会混淆两套包名。[发布说明](https://github.com/longbridge/gpui-fast/blob/bf313b428e28f0bfd14aaa7bbd25c4372f4052de/README.md#using-it) · [核心发布清单](https://docs.rs/crate/gpui-fast/0.1.17/source/Cargo.toml) · [平台发布清单](https://docs.rs/crate/gpui-fast-platform/0.1.17/source/Cargo.toml) · [发布准备脚本](https://github.com/longbridge/gpui-fast/blob/bf313b428e28f0bfd14aaa7bbd25c4372f4052de/script/prepare-release.ts)

~~~toml
[dependencies]
gpui = { package = "gpui-fast", version = "=0.1.17", default-features = false }

[target.'cfg(target_os = "macos")'.dependencies]
gpui_platform = { package = "gpui-fast-platform", version = "=0.1.17", default-features = false, features = ["font-kit", "runtime_shaders"] }

[target.'cfg(target_os = "linux")'.dependencies]
gpui_platform = { package = "gpui-fast-platform", version = "=0.1.17", default-features = false, features = ["wayland", "x11"] }
~~~

- **macOS 文字需要显式启用平台的 `font-kit`。** 平台转发到 macOS 和 Apple 包。关闭时 `MacPlatform` 会创建 `NoopTextSystem`，窗口能建立也可能没有文字。应用会注册四款内嵌 JetBrains Mono 字体，另通过 `AssetSource` 读取内嵌 SVG。要用中英文、emoji 回退和图标验证实际绘制。[MacPlatform](https://github.com/longbridge/gpui-fast/blob/bf313b428e28f0bfd14aaa7bbd25c4372f4052de/crates/gpui_macos/src/platform.rs#L219-L235) · [注册字体](../../../src/settings.rs#L225-L243) · [图标](../../../src/ui/icon.rs#L24-L41)
- **Linux 两个 feature 都要开在平台包上。** 平台依赖的 Linux 包关闭默认 feature，`wayland`、`x11` 分别选入原生协议及 wgpu 渲染。旧核心包默认 feature 所连的 Blade 依赖不同。须重新验证发行版依赖、驱动与渲染，尤其是应用的客户区装饰和窗口管理器提供装饰两条路径。[旧核心清单](https://docs.rs/crate/gpui/0.2.2/source/Cargo.toml) · [新平台清单](https://docs.rs/crate/gpui-fast-platform/0.1.17/source/Cargo.toml) · [Linux 后端清单](https://github.com/longbridge/gpui-fast/blob/bf313b428e28f0bfd14aaa7bbd25c4372f4052de/crates/gpui_linux/Cargo.toml#L15-L52) · [窗口边框](../../../src/window_frame.rs)
- **工具链先按上游 `1.98.1` 试编。** 本仓库固定 `1.90.0`，上游平台构造器使用 `cfg_select!`。下表的编译器探针确认 1.90 stable 对该宏报错。上游工具链文件提供可复现的起点，整棵依赖树的最低 Rust 版本仍待测。顶层两个包精确固定后也要提交验证过的 `Cargo.lock`，发布清单对多个子包用 `^0.1.17`，单靠顶层等号无法固定未来解析结果。[本仓库工具链](../../../rust-toolchain.toml) · [上游工具链](https://github.com/longbridge/gpui-fast/blob/bf313b428e28f0bfd14aaa7bbd25c4372f4052de/rust-toolchain.toml) · [宏调用](https://github.com/longbridge/gpui-fast/blob/bf313b428e28f0bfd14aaa7bbd25c4372f4052de/crates/gpui_platform/src/gpui_platform.rs#L61-L81) · [发布清单](https://docs.rs/crate/gpui-fast-platform/0.1.17/source/Cargo.toml)

## 应用接口与行为

| 应用现场 | 对照 0.1.17 源码的结论 | 验收动作 |
|---|---|---|
| 启动、URL、菜单与焦点 | `Application::new()` 改由 `gpui_platform::application()` 创建。`with_assets`、`on_open_urls`、`open_window` 有对应入口。`Window::focus(&handle)` 多了 `cx` 参数。`Menu { name, items }` 的构造缺新增的 `disabled` 字段，可用菜单构造器。[应用入口及菜单](../../../src/main.rs) · [平台构造器](https://github.com/longbridge/gpui-fast/blob/bf313b428e28f0bfd14aaa7bbd25c4372f4052de/crates/gpui_platform/src/gpui_platform.rs#L16-L22) · [App](https://github.com/longbridge/gpui-fast/blob/bf313b428e28f0bfd14aaa7bbd25c4372f4052de/crates/gpui/src/app.rs) · [Window](https://github.com/longbridge/gpui-fast/blob/bf313b428e28f0bfd14aaa7bbd25c4372f4052de/crates/gpui/src/window.rs) · [Menu](https://github.com/longbridge/gpui-fast/blob/bf313b428e28f0bfd14aaa7bbd25c4372f4052de/crates/gpui/src/platform/app_menu.rs) | 从 Finder／Linux 启动器打开，输入首字，切换标签后再输入。验证 Edit 菜单、快捷键、已有窗口和冷启动的会话深链。 |
| 标题栏及系统对话框 | 应用设置透明标题栏、红绿灯位置、Linux app_id 与装饰。Mac 拖窗绕过旧版空实现，读 raw-window-handle 的 AppKit view。Fast 保留窗口句柄接口，Mac 的 `start_window_move` 已调用 `performWindowDragWithEvent:`。目录选择器还依赖应用自己的 AppKit 回调。[创建窗口](../../../src/main.rs#L137-L158) · [应用平台桥接](../../../src/platform.rs) · [上游拖窗](https://github.com/longbridge/gpui-fast/blob/bf313b428e28f0bfd14aaa7bbd25c4372f4052de/crates/gpui_macos/src/fast/composition.rs) · [上游窗口选项](https://github.com/longbridge/gpui-fast/blob/bf313b428e28f0bfd14aaa7bbd25c4372f4052de/crates/gpui/src/platform.rs) | Mac 实测拖动、按钮命中、全屏、目录选择／取消后的焦点。Linux 在两种会话中各测拖动、边缘缩放、最大化与平铺。确认后再决定能否去掉旧桥接。 |
| 编辑器、选区、剪贴板 | 自绘 `TextElement` 在 paint 时用 `Window::handle_input` 注册 `ElementInputHandler`。Fast 仍有 UTF-16 选区／标记文本、范围矩形、点到字符的回调。新增方法有默认实现，例如默认 paste 处理文本，另有通知原生文本服务内容／选区变化的入口。应用自有图片／文件粘贴处理。`save_clipboard_image` 穷举旧格式，Fast 新增 `Ico`、`Pnm`，必须明确处理。[应用输入](../../../src/text_input.rs#L1180-L1310) · [图片粘贴](../../../src/attachments.rs#L62-L97) · [输入 trait](https://github.com/longbridge/gpui-fast/blob/bf313b428e28f0bfd14aaa7bbd25c4372f4052de/crates/gpui/src/input.rs) · [焦点输入注册](https://github.com/longbridge/gpui-fast/blob/bf313b428e28f0bfd14aaa7bbd25c4372f4052de/crates/gpui/src/window.rs#L5790-L5845) · [ImageFormat](https://github.com/longbridge/gpui-fast/blob/bf313b428e28f0bfd14aaa7bbd25c4372f4052de/crates/gpui/src/platform.rs#L3334-L3362) | 用中日韩输入法组字、移动候选窗并提交／取消，测多行折行、emoji 前后删除、反向选区、鼠标拖选、撤销、文本和截图粘贴。切换输入框后新焦点独占按键；按 Enter 选候选时不能误发消息。 |
| 会话列表、插件和局部刷新 | 应用聊天滚动依赖 `ListState`，选择文本与插件视图有自己的绘制、鼠标／键盘事件。Fast 缓存视图，未被跟踪的外部状态改变时需通知所属实体，否则可能看到旧帧。在 paint 中无条件通知也会抵消缓存收益。[会话列表](../../../src/chat.rs) · [文本选择](../../../src/selectable.rs) · [插件焦点和按键](../../../src/chat/plugin_view.rs) · [保留模式契约](https://github.com/longbridge/gpui-fast/blob/bf313b428e28f0bfd14aaa7bbd25c4372f4052de/docs/retained-mode.md#what-an-application-needs-to-know) | 流式输出时滚动离底再回底，切标签、换主题、插件更新帧、取得／交还焦点，检查遗漏刷新和重复按键。若出现旧帧，用 `GPUI_VIEW_RETENTION=0` 对照定位。 |

## Metal 与交付物

**可先保留现有运行时编译策略。** 旧应用在核心包上设 `runtime_shaders`。Fast 改为 platform → macOS → Apple 包转发。Apple 构建脚本用 cbindgen 生成 `scene.h`，然后把 shader 源码和头拼入 `stitched_shaders.metal`，由渲染器 `include_str!` 并通过 Metal 设备编译。关闭该 feature 时，脚本依次调用 `xcrun … metal` 与 `metallib`，再由 `include_bytes!` 把结果编入二进制。两条路径都无需另把 shader 文件复制到 .app。发布包的脚本从随包附带的 `src/fast/release_gpui` 取生成绑定的源文件，Git 工作区走相邻核心目录。运行时方案仍依赖可用的 Metal 设备与驱动，须测首次创建窗口和文字渲染。[旧配置](../../../Cargo.toml#L9-L12) · [Mac feature](https://github.com/longbridge/gpui-fast/blob/bf313b428e28f0bfd14aaa7bbd25c4372f4052de/crates/gpui_macos/Cargo.toml#L15-L21) · [发布包 build.rs](https://docs.rs/crate/gpui-fast-apple/0.1.17/source/build.rs) · [编译库的两条路径](https://github.com/longbridge/gpui-fast/blob/bf313b428e28f0bfd14aaa7bbd25c4372f4052de/crates/gpui_apple/src/metal_renderer.rs) · [发布时复制源文件](https://github.com/longbridge/gpui-fast/blob/bf313b428e28f0bfd14aaa7bbd25c4372f4052de/script/prepare-release.ts#L130-L155)

| 现有交付契约 | 迁移验收（待执行） |
|---|---|
| macOS release 在 macos-15 跑 `cargo test --locked`。bundle 脚本执行 `cargo build --release`，组装可执行文件、Info.plist、图标、字体许可和存在时的插件。分发流程执行 Developer ID 签名、公证、打包 `Pi-GPUI-<版本>-aarch64.zip`，生成 `pi-compat.json`。[bundle](../../../script/bundle.sh) · [元数据和系统权限](../../../resources/Info.plist) · [release](../../../.github/workflows/release.yml) | 在无额外 Metal Toolchain 的 Mac 对锁文件执行 release 构建，检查构建后 lock 无漂移。在源码树外 `open` 已签名公证的 .app，检查签名／stapling、字体图标、URL scheme、插件与权限提示；在 macOS 13 与当前系统实测。验证 `--pi-compat` 和测试安装的升级重启。[公证脚本](../../../script/notarize.sh) · [更新器](../../../src/updater.rs) |
| Linux CI 在 Ubuntu 22.04 x86_64／arm64 安装系统依赖、执行 `cargo test --locked` 并构建 tarball。内含 `pi-gpui.app/bin` 和 `share` 树。安装器和更新器依赖现有包名与目录。[CI](../../../.github/workflows/ci.yml) · [系统依赖](../../../script/linux-deps.sh) · [打包](../../../script/bundle-linux.sh) · [安装器](../../../site/install.sh) | 两架构各跑已有 CI、检查缺失动态库及 glibc 符号版本，在干净系统解包安装、启动并走升级重启。在有 GPU／驱动的 X11 与 Wayland 桌面走一遍窗口、输入法、滚动、剪贴板及再次启动时的单实例转发。保留后端、架构与驱动记录。 |

## 实际检查与决策门槛

| 本次检查 | 结果与边界 |
|---|---|
| `git ls-remote --tags https://github.com/longbridge/gpui-fast.git`，检出 `v0.1.17^{}`。下载 crates.io 核心、平台、Mac、Apple、Linux 包及旧 gpui 包阅读源码。 | 核心与平台 0.1.17 的索引均为 `yanked=false`，下载包的 SHA-256 分别为 `4481f1c7729d23de02a516384278293a4f95a3bdd4b735fc474076675fed9634`、`1360d84fd3872d05c73df7bd9a6ed95deb06cb643fff5d6bc08dc49f869b2fdb`，与索引相同。[核心索引](https://index.crates.io/gp/ui/gpui-fast) · [平台索引](https://index.crates.io/gp/ui/gpui-fast-platform) |
| 在 macOS arm64 临时目录复制本项目 manifest／lock，代入上面的别名及 feature，运行 `cargo +1.100.0 generate-lockfile`。 | 依赖解析成功，临时 lock 中的核心、平台、macOS、Apple、Linux 均为 0.1.17。跨目标锁也包含平台包 iOS 分支依赖的 `gpui-pre-ios 0.3.12`。要用 `cargo tree --target <目标> -e features` 确认实际桌面构建的 feature 和核心来源。 |
| 在仓库工具链用 `rustc +1.90.0 --edition=2024` 编译含 `cfg_select! { target_os = "macos" => {} _ => {} }` 的最小 main；在同一台机器运行 `xcrun --find metal`。 | 前者返回 `E0658: use of unstable library feature cfg_select`，后者找不到 metal。两项是独立环境探针。本次没有编译迁移后的应用、生成发行包，或测量启动时间、内存和体积。 |

**放行条件。** 先在候选工具链以同一份 lock 跑三个发布目标的 `cargo check --locked --all-targets`、`cargo test --locked` 和 release 构建，记录其余编译差异，再用安装好的包完成上表交互和更新验收。Apple Silicon 性能单独测量首次可输入、静置 10 秒应用自身内存及安装后 .app 体积。现有脚本计预热后的首次可见窗口，上游性能表用自己的工作负载，两者都不能给出本应用首次输入就绪的数值。[本项目基准脚本](../../../script/bench/bench.sh) · [上游性能测试与方法](https://github.com/longbridge/gpui-fast/blob/bf313b428e28f0bfd14aaa7bbd25c4372f4052de/README.md#performance-vs-gpui)
