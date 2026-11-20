# 哪些 Mermaid 渲染方案适合聊天中的流程图

**优先验证 Rust 库 `mermaid-rs-renderer`。** 对聊天里常见的节点、分支和子图，它有可直接调用的解析、布局和 SVG 输出路径，无需让用户安装浏览器或 Node。若验收要求是按官方 Mermaid 语法与布局展示，选随包的 Mermaid JS 加本地 WebView。官方 CLI `mmdc` 适合作兼容性对照或显式导出，逐块启动浏览器与流式聊天的更新方式不匹配。这个排序依据接入路径和依赖，三者在本应用的性能差值均待测。[Rust API][rust-api] · [官方 JS API][js-api] · [CLI 实现][cli-src] · [本项目响应性要求][principles]

## 接入现有 Markdown 的位置

当前 assistant 消息的 `text_delta` 会追加到正文。绘制消息行时，应用以行索引和全文哈希复用 Markdown 节点，正文变化就重新解析。围栏信息的第一个词保存在 `Node::Code { language, text }`，渲染端统一交给 `code_block`，已有选择、横向滚动和复制原文。建议只在该分支识别语言标记 `mermaid`，再检查正文图种 `flowchart` 或 `graph`。其他代码和图种继续显示原文。图像展示可参考插件帧现用的 `gpui::img(ImageSource::Render(...))`，动态 SVG 到彩色图像、字体和缩放仍需单独验证。[流式消息][chat-delta] · [消息行缓存][chat-cache] · [围栏解析][md-parse] · [代码块绘制][md-render] · [插件帧][frame]

## 三种本地路径

| 路径 | 语法与错误 | CPU、离线与打包 | 适合的位置 |
|---|---|---|---|
| **Rust 库** `mermaid-rs-renderer` 0.3.1 | 自有解析器识别两种图头、边链、子图、方向和部分样式。`render_strict` 返回 SVG 或结构化解析错误。解析器会跳过部分声明，也存在未匹配行继续处理的路径，成功返回 SVG 不足以证明完整表达了输入。[解析器][rust-parser] · [严格 API][rust-api] · [错误用例][rust-errors] | 在本进程做解析、字体度量、布局和 SVG 序列化，显示位图还要栅格化。SVG 库模式可关闭默认 features，省去 CLI 和可选 PNG 链。本版 PNG feature 引入 resvg/usvg 0.47，本应用锁定的 GPUI 链是 0.45.1，开启时可能并存两版。字体度量会用到本机字体，中文效果要在目标系统验。[crate 清单][rust-cargo] · [字体度量][fonts] · [本应用锁文件][lock] | 默认候选，承诺经过对照的静态子集。 |
| **系统 WebView + 随包 Mermaid JS**，参照 Mermaid 11.17.2、Wry | 使用官方解析和布局。`mermaid.parse(text, {suppressErrors:true})` 对无效输入返回 `false`，`render(id,text)` 异步返回 SVG 与可选事件绑定。两个入口都接收完整字符串，显示仍须捕获渲染错误。渲染实现读写 DOM，仅嵌 JS 引擎还缺浏览器布局环境。[官方接口][js-api] · [渲染实现][js-render] | 有 JS 模块加载、DOM 测量、布局与绘制成本。随包带模块及所用分块、样式和字体；首次用图时再建宿主。macOS 可用系统 WebKit。Linux 的 Wry 用 WebKitGTK，原生子窗口路径仅支持 X11，兼顾 Wayland 要走 GTK 容器并处理事件循环。需验证系统依赖、聊天行裁剪滚动和离屏渲染可行性。[Mermaid 分发说明][usage] · [Wry 平台说明][wry] | 官方兼容优先时，用一个可复用的受限渲染页生成静态图。避免在虚拟列表中为每块维护一个活页面。 |
| **本地官方 CLI** `mmdc` 11.17.1 | 可从 stdin 读定义，输出 SVG/PNG/PDF，内部在浏览器页调用官方 `mermaid.render`。对照结果要同时固定实际安装的 Mermaid 版本、配置和字体，CLI 的包声明使用版本范围。子进程失败或超时可按块回退。[CLI 用法][cli] · [包声明][cli-pkg] · [调用链][cli-src] | 需供应或明确要求 Node、CLI 依赖及 Puppeteer 可启动的兼容浏览器。在干净机器上首次断网可用，要求这些组件已在本地。一次 CLI 调用涉及进程、浏览器、页面及图像传递；复用常驻浏览器可以摊销启动，但引入额外生命周期和驻留成本。当前 macOS/Linux 打包脚本只构建 Rust 应用并复制项目资源。[CLI 实现][cli-src] · [macOS 打包][bundle-mac] · [Linux 打包][bundle-linux] | 开发时作为官方输出参照。若提供独立导出，可由用户触发并在后台运行。 |

## 应承诺哪些语法

建议首批验收用同一份文本分别送入锁版的官方引擎和 Rust 候选。比较节点、箭头方向、边标签、分组、中文可读性及越界情况，像素完全一致并非此子集的验收目标。[官方流程图语法][syntax] · [Rust 解析器][rust-parser]

~~~mermaid
graph LR
  subgraph S [校验]
    A["收到消息"] --> B{完整吗}
  end
  B -->|是| C[渲染]
  B -.->|否| D[等待]
~~~

- **基础样本**覆盖 `graph LR` 与 `flowchart TD` 两种图头，再测 `TB/BT/RL`、矩形/圆角/菱形、链式和带标签的边、带引号的 Unicode 文本、注释和子图。单独测跨子图连线与子图内部方向，官方对外连边影响子图方向有专门说明。[官方方向、连线与子图示例][syntax]
- **兼容性分界**用 `classDef/style/linkStyle`、嵌套子图、Markdown/HTML 标签及 `@{ shape: ... }` 扩展形状做对照。Rust 源码有样式分支，但这不能推导所有属性或新形状都兼容。尚未验证、可能丢节点或改变流程的构造，整块显示源码；若这些构造属于必需语法，转用官方引擎并明确静态资源规则。[Rust 样式处理][rust-parser] · [官方形状与标签][syntax]

## 增量输入、失败和资源访问

1. **先显示可复制的代码。** 项目测试证明围栏尚未闭合就可能产出代码节点，现有节点只存语言和文本。最简单的触发条件是等消息完成；若希望在长回复中提前出图，需额外取得该围栏实际闭合的证据。半条 `A -->`、未结束的标签或缺 `end` 的子图在流式阶段均保持原文，不逐 token 报错。最终无效时保留原文并提示该块原因，后续消息正常渲染。[现有未闭合围栏测试][md-test] · [Rust 错误用例][rust-errors] · [官方错误接口][js-api]
2. **按块调度后台任务。** 只有可见且内容已稳定的块才进入有界队列，同一块只提交最新版本。缓存键包含块内容、渲染器版本、主题和字体，位图另含有效尺寸/像素比。主题切换、消息更改、会话切换时，丢弃过期任务结果。同一失败内容可缓存诊断，避免滚动反复触发布局。限制源码长度、图规模及输出像素，超限保留原文。原生同步调用的超时不能自动终止已在运行的 CPU 工作，需在提交前限额。[同步 Rust 调用链][rust-api] · [消息缓存][chat-cache] · [项目队列要求][principles]
3. **断网与禁外部访问分别验收。** 官方语法允许 URL 图片、图标包、Font Awesome CSS 和 `click`，Rust 解析器也记录节点链接。对聊天文本建议只接收本地静态资源，拒绝图内的远程图片、任意文件路径、外部 CSS/字体、导航和回调；遇到这些构造就保留原文。WebView/CLI 应在渲染开始前限制请求和文件访问，固定宿主配置、禁用图内配置覆盖。官方 `securityLevel: strict` 编码 HTML 并禁用点击，网络和文件访问仍需由宿主约束。Rust 路径也需检查输出 SVG 的引用并约束栅格器资源解析。以断网、空缓存环境测试普通图，再用外部图片与本地文件引用确认没有访问尝试。[官方资源语法][syntax] · [安全配置][usage] · [图内指令][directives] · [Rust 链接处理][rust-parser]

## 哪些需要实测

| 决策项 | 建议取证方式 |
|---|---|
| 能否采用 Rust 子集 | 在 macOS/Linux 上用同一批基础、扩展及非法文本逐项记录“正确出图 / 显示原文 / 错画”。保留官方对照图。把示例按字符分段输入，覆盖中断、恢复、未闭合围栏、渲染中切换会话。静默漏掉节点、边或文字应阻止该语法进入承诺范围。 |
| 聊天交互成本 | 同机 release 构建对照原版。分别测无图启动、首次见图到首帧、重复打开缓存图，以及流式输出并滚动长会话。按节点数、边密度、子图深度取样，记录解析/布局/栅格化、UI 帧耗时、CPU 时间和命中率。分别记录冷、热状态。 |
| 包体、内存、部署 | 对同平台同架构的压缩发布物和解包目录求差，列出另需安装的系统组件。测无图空闲、首次出图、同时多图及离开会话后的进程树内存，将 WebView/浏览器助手计入。用干净机器验证离线首次渲染、中文字体回退及 Linux X11/Wayland。仓库已有启动和内存基准可复用测量方法，但其旧版本结果及上游库的单图 benchmark 都不能给出本改动的冷启动、包体或内存增量。[项目基准方法][bench] · [Rust 上游基准条件][rust-readme] |

[principles]: ../../../docs/APP_PRINCIPLES.md
[chat-delta]: ../../../src/chat.rs#L1991
[chat-cache]: ../../../src/chat.rs#L2213-L2235
[md-parse]: ../../../src/markdown.rs#L134-L145
[md-render]: ../../../src/markdown.rs#L423-L532
[md-test]: ../../../src/markdown.rs#L737-L744
[frame]: ../../../src/chat/plugin_view.rs#L92
[lock]: ../../../Cargo.lock#L4171-L4173
[bundle-mac]: ../../../script/bundle.sh
[bundle-linux]: ../../../script/bundle-linux.sh
[bench]: ../../../docs/BENCHMARKS.md
[rust-readme]: https://github.com/1jehuang/mermaid-rs-renderer/blob/v0.3.1/README.md
[rust-api]: https://github.com/1jehuang/mermaid-rs-renderer/blob/v0.3.1/src/lib.rs#L222-L279
[rust-parser]: https://github.com/1jehuang/mermaid-rs-renderer/blob/v0.3.1/src/parser.rs#L336-L456
[rust-errors]: https://github.com/1jehuang/mermaid-rs-renderer/blob/v0.3.1/tests/parse_errors.rs
[rust-cargo]: https://github.com/1jehuang/mermaid-rs-renderer/blob/v0.3.1/Cargo.toml#L39-L58
[fonts]: https://github.com/1jehuang/mermaid-rs-renderer/blob/v0.3.1/src/text_metrics.rs
[usage]: https://github.com/mermaid-js/mermaid/blob/mermaid%4011.17.2/packages/mermaid/src/docs/config/usage.md
[directives]: https://github.com/mermaid-js/mermaid/blob/mermaid%4011.17.2/packages/mermaid/src/docs/config/directives.md
[syntax]: https://github.com/mermaid-js/mermaid/blob/mermaid%4011.17.2/packages/mermaid/src/docs/syntax/flowchart.md
[js-api]: https://github.com/mermaid-js/mermaid/blob/mermaid%4011.17.2/packages/mermaid/src/mermaid.ts#L343-L410
[js-render]: https://github.com/mermaid-js/mermaid/blob/mermaid%4011.17.2/packages/mermaid/src/mermaidAPI.ts#L476-L657
[wry]: https://github.com/tauri-apps/wry/blob/259b0969f367b69ee2a54dbdaef033e24719afdd/README.md
[cli]: https://github.com/mermaid-js/mermaid-cli/blob/11.17.1/README.md
[cli-pkg]: https://github.com/mermaid-js/mermaid-cli/blob/11.17.1/package.json
[cli-src]: https://github.com/mermaid-js/mermaid-cli/blob/11.17.1/src/index.js
