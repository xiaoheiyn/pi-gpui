use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::Value;

use super::*;

#[derive(Clone, Debug, PartialEq, Eq)]
struct ChildKey {
    call: String,
    run: String,
    child: String,
}

#[derive(Debug)]
struct Child {
    key: ChildKey,
    name: String,
    task: String,
    running: bool,
    hidden: bool,
    receipt: Arc<str>,
    file: Option<PathBuf>,
}

struct PendingInspection {
    child: usize,
    request: String,
    rpc: String,
}

pub(super) struct ChildTranscript {
    index: usize,
    messages: Vec<Value>,
    running: bool,
    pub(super) rows: Vec<Row>,
    pub(super) list: ListState,
}

impl ChildTranscript {
    fn new(index: usize) -> Self {
        Self {
            index,
            messages: Vec::new(),
            running: false,
            rows: Vec::new(),
            list: ListState::new(0, ListAlignment::Bottom, px(LIST_OVERDRAW)),
        }
    }

    fn update(&mut self, messages: Vec<Value>, running: bool) -> bool {
        if self.messages == messages && self.running == running {
            return false;
        }
        let mut rows = history_rows(&messages, running);
        // 子会话记录里的路径不能为工具日志授权。
        for row in &mut rows {
            if let Row::Tool { log, .. } = row {
                log.file = None;
            }
        }
        if rows.len() >= self.rows.len() && messages.starts_with(&self.messages) {
            for (previous, row) in self.rows.iter_mut().zip(&mut rows) {
                match (previous, row) {
                    (
                        Row::Assistant {
                            thought_overrides: old,
                            ..
                        },
                        Row::Assistant {
                            thought_overrides, ..
                        },
                    ) => {
                        *thought_overrides = std::mem::take(old);
                    }
                    (Row::Tool { expanded: old, .. }, Row::Tool { expanded, .. }) => {
                        *expanded = *old;
                    }
                    (Row::Summary { expanded: old, .. }, Row::Summary { expanded, .. }) => {
                        *expanded = *old;
                    }
                    _ => {}
                }
            }
            let previous = self.rows.len();
            self.list.splice(previous..previous, rows.len() - previous);
            self.list.remeasure_items(0..previous);
        } else {
            self.list.reset(rows.len());
        }
        self.messages = messages;
        self.running = running;
        self.rows = rows;
        true
    }
}

#[derive(Default)]
pub(super) struct Children {
    calls: HashMap<String, Value>,
    jobs: HashMap<String, String>,
    running_jobs: std::collections::HashSet<String>,
    snapshot: Option<Value>,
    inspection: Option<PendingInspection>,
    next_inspection: u64,
    children: Vec<Child>,
    pub(super) active: Option<ChildTranscript>,
    read: Option<Task<()>>,
}

#[derive(Clone, Copy)]
pub(super) enum ResultPhase {
    Progress,
    Finished,
}

impl Children {
    pub(super) fn start(&mut self, call: &str, args: &Value) {
        self.calls.insert(call.into(), args.clone());
    }

    pub(super) fn result(&mut self, call: &str, result: &Value, phase: ResultPhase) {
        let finished = matches!(phase, ResultPhase::Finished);
        let Some(args) = self.calls.get(call).cloned() else {
            return;
        };
        let details = &result["details"];
        let receipt: Arc<str> = Arc::from(content_text(result));
        let Some(run) = details["runId"].as_str() else {
            if finished {
                for child in self
                    .children
                    .iter_mut()
                    .filter(|child| child.key.call == call)
                {
                    child.running = false;
                    child.receipt = receipt.clone();
                }
            }
            return;
        };
        let inventory = &details["workflowChildren"];
        if inventory["version"] == 1 && inventory["workflowRunId"] == run {
            for row in inventory["children"].as_array().into_iter().flatten() {
                let Some(id) = row["childId"].as_str() else {
                    continue;
                };
                if row["state"] == "pending" {
                    continue;
                }
                let name = row["agent"].as_str().unwrap_or(id);
                let child = self.child(call, run, id, name, "");
                child.name = name.into();
                child.running = !terminal(&row["state"]);
                if let Some(file) = row["sessionFile"].as_str() {
                    child.file = Some(file.into());
                }
                if let Some(tool) = row["activity"]["currentTool"].as_str() {
                    child.receipt = Arc::from(format!("Running {tool}…"));
                }
            }
        }
        if let Some(job) = details["asyncId"].as_str() {
            let owner = self.jobs.entry(job.into()).or_insert_with(|| call.into());
            if owner != call {
                return;
            }
            self.running_jobs.insert(job.into());
            if let Some(agent) = args["agent"].as_str() {
                let child = self.child(
                    call,
                    job,
                    job,
                    agent,
                    args["task"].as_str().unwrap_or_default(),
                );
                child.receipt = receipt;
            }
            if let Some(snapshot) = self.snapshot.take() {
                self.apply_snapshot(&snapshot);
                self.snapshot = Some(snapshot);
            }
            return;
        }
        for row in details["progress"]
            .as_array()
            .into_iter()
            .flatten()
            .chain(details["results"].as_array().into_iter().flatten())
        {
            let (Some(index), Some(agent)) = (row["index"].as_u64(), row["agent"].as_str()) else {
                continue;
            };
            let task = args["tasks"][index as usize]["task"]
                .as_str()
                .or_else(|| args["chain"][index as usize]["task"].as_str())
                .or_else(|| args["task"].as_str())
                .or_else(|| {
                    row["task"]
                        .as_str()
                        .filter(|task| *task != "[prompt redacted]")
                })
                .unwrap_or_default();
            let index_id = index.to_string();
            let child_id = row["workflowKey"].as_str().unwrap_or(&index_id);
            let child = self.child(call, run, child_id, agent, task);
            child.running =
                !finished && !terminal(&row["status"]) && !terminal(&row["progress"]["status"]);
            if let Some(file) = row["sessionFile"]
                .as_str()
                .or(row["progress"]["sessionFile"].as_str())
            {
                child.file = Some(file.into());
            }
            if finished {
                child.receipt = row["finalOutput"]
                    .as_str()
                    .or(row["output"].as_str())
                    .filter(|text| !text.is_empty())
                    .map(Arc::from)
                    .unwrap_or_else(|| receipt.clone());
            } else if let Some(lines) = row["recentOutput"].as_array() {
                child.receipt = Arc::from(
                    lines
                        .iter()
                        .filter_map(Value::as_str)
                        .collect::<Vec<_>>()
                        .join("\n"),
                );
            }
        }
        if finished {
            for child in self
                .children
                .iter_mut()
                .filter(|child| child.key.call == call && child.key.run == run)
            {
                child.running = false;
                if child.receipt.is_empty() {
                    child.receipt = receipt.clone();
                }
            }
        }
    }

    fn child(&mut self, call: &str, run: &str, id: &str, name: &str, task: &str) -> &mut Child {
        let index = self
            .children
            .iter()
            .position(|child| {
                child.key.call == call && child.key.run == run && child.key.child == id
            })
            .unwrap_or_else(|| {
                self.children.push(Child {
                    key: ChildKey {
                        call: call.into(),
                        run: run.into(),
                        child: id.into(),
                    },
                    name: name.into(),
                    task: task.into(),
                    running: true,
                    hidden: false,
                    receipt: Arc::from(""),
                    file: None,
                });
                self.children.len() - 1
            });
        let child = &mut self.children[index];
        if child.task.is_empty() && !task.is_empty() {
            child.task = task.into();
        }
        child
    }

    pub(super) fn restore(&mut self, messages: &[Value]) {
        for message in messages {
            match message["role"].as_str() {
                Some("assistant") => {
                    for block in message["content"].as_array().into_iter().flatten() {
                        if block["type"] == "toolCall"
                            && block["name"] == "subagent"
                            && let Some(id) = block["id"].as_str()
                        {
                            self.start(id, &block["arguments"]);
                        }
                    }
                }
                Some("toolResult") => {
                    if let Some(id) = message["toolCallId"].as_str() {
                        self.result(id, message, ResultPhase::Finished);
                    }
                }
                Some("custom") => self.notice(message),
                _ => {}
            }
        }
        for child in &mut self.children {
            child.hidden = !child.running;
        }
    }

    pub(super) fn widget(&mut self, record: &Value) {
        let Some(payload) = record["widgetLines"]
            .as_array()
            .or(record["lines"].as_array())
            .and_then(|lines| {
                lines
                    .iter()
                    .filter_map(Value::as_str)
                    .find_map(|line| line.strip_prefix("PI_SUBAGENT_ASYNC_JSON:"))
            })
        else {
            return;
        };
        if payload.len() > 256 * 1024 {
            return;
        }
        let Ok(snapshot) = serde_json::from_str::<Value>(payload) else {
            return;
        };
        if snapshot["kind"] != "pi-subagents.async-status-snapshot" || snapshot["version"] != 1 {
            return;
        }
        self.apply_snapshot(&snapshot);
        self.snapshot = Some(snapshot);
    }

    fn apply_snapshot(&mut self, snapshot: &Value) {
        for run in snapshot["runs"].as_array().into_iter().flatten() {
            let Some(run_id) = run["id"].as_str() else {
                continue;
            };
            let Some(call) = self.jobs.get(run_id).cloned() else {
                continue;
            };
            if terminal(&run["state"]) {
                self.running_jobs.remove(run_id);
            }
            if self.children.iter().any(|child| {
                child.key.call == call && child.key.run == run_id && child.key.child == run_id
            }) {
                self.apply_async_child(&call, run_id, run_id, run);
            } else {
                self.async_row(&call, run_id, run);
            }
        }
    }

    fn async_row(&mut self, call: &str, run: &str, row: &Value) {
        let Some(id) = row["id"].as_str() else { return };
        let Some(children) = row["children"].as_array().filter(|items| !items.is_empty()) else {
            // 顶层工作流可能先到，子任务列表随后才出现。
            if id != run || row["kind"] == "single" {
                self.apply_async_child(call, run, id, row);
            }
            return;
        };
        for child in children.iter().filter(|child| child["kind"] != "host-step") {
            self.async_row(call, run, child);
        }
    }

    fn apply_async_child(&mut self, call: &str, run: &str, id: &str, row: &Value) {
        let child = self.child(
            call,
            run,
            id,
            row["agent"]
                .as_str()
                .or(row["label"].as_str())
                .unwrap_or("subagent"),
            "",
        );
        child.running = !terminal(&row["state"]);
        if let Some(file) = row["sessionFile"].as_str() {
            child.file = Some(file.into());
        }
        if child.running
            && let Some(progress) = row["progressText"].as_str()
        {
            child.receipt = Arc::from(progress);
        }
    }

    pub(super) fn inspection_reply(&mut self, record: &Value) {
        let Some(inspection) = &self.inspection else {
            return;
        };
        let Some(payload) = record["widgetLines"].as_array().and_then(|lines| {
            lines
                .iter()
                .filter_map(Value::as_str)
                .find_map(|line| line.strip_prefix("PI_SUBAGENT_INSPECT_JSON:"))
        }) else {
            return;
        };
        if payload.len() > 64 * 1024 {
            return;
        }
        let Ok(reply) = serde_json::from_str::<Value>(payload) else {
            return;
        };
        if reply["kind"] != "pi-subagents.inspect-reply"
            || reply["version"] != 1
            || reply["requestId"].as_str() != Some(&inspection.request)
        {
            return;
        }
        let Some(inspection) = self.inspection.take() else {
            return;
        };
        // 旧页面的回包可以晚于切页，不能写入当前记录。
        let Some(active) = self
            .active
            .as_mut()
            .filter(|active| active.index == inspection.child)
        else {
            return;
        };
        let child = &mut self.children[inspection.child];
        if let Some(task) = reply["task"].as_str()
            && child.task.is_empty()
        {
            child.task = task.into();
        }
        if terminal(&reply["status"]) {
            child.running = false;
        }
        let messages = inspection_messages(&reply);
        if !messages.is_empty() {
            active.update(messages, child.running);
        }
    }

    pub(super) fn is_inspection_response(&mut self, record: &Value) -> bool {
        if let Some(inspection) = &self.inspection
            && record["id"].as_str() == Some(&inspection.rpc)
        {
            if record["success"] != true {
                self.inspection = None;
            }
            true
        } else {
            false
        }
    }

    pub(super) fn notice(&mut self, message: &Value) {
        if message["customType"] != "subagent-notify" {
            return;
        }
        let receipt: Arc<str> = Arc::from(sessions::message_text(message));
        for run in message["details"]["runs"].as_array().into_iter().flatten() {
            let Some(id) = run["runId"].as_str() else {
                continue;
            };
            let Some(call) = self.jobs.get(id).cloned() else {
                continue;
            };
            self.running_jobs.remove(id);
            if let Some(items) = run["childRuns"]
                .as_array()
                .filter(|items| !items.is_empty())
            {
                for item in items {
                    let Some(child_id) = item["workflowKey"].as_str().or(item["runId"].as_str())
                    else {
                        continue;
                    };
                    let child = self.child(
                        &call,
                        id,
                        child_id,
                        item["agent"].as_str().unwrap_or("subagent"),
                        "",
                    );
                    child.receipt = receipt.clone();
                    if let Some(file) = item["sessionFile"].as_str() {
                        child.file = Some(file.into());
                    }
                }
            } else {
                let child = self.child(
                    &call,
                    id,
                    id,
                    run["agent"].as_str().unwrap_or("subagent"),
                    "",
                );
                child.file = run["sessionFile"]
                    .as_str()
                    .or_else(|| {
                        (run["sessionLabel"] == "Session file")
                            .then(|| run["sessionValue"].as_str())
                            .flatten()
                    })
                    .map(PathBuf::from);
            }
            for child in self
                .children
                .iter_mut()
                .filter(|child| child.key.call == call && child.key.run == id)
            {
                child.running = false;
                child.receipt = receipt.clone();
            }
        }
    }

    fn label(&self, index: usize) -> String {
        let child = &self.children[index];
        let status = if child.running {
            "In progress"
        } else {
            "Completed"
        };
        format!("#{} {} · {status}", index + 1, child.name)
    }

    fn active_index(&self) -> Option<usize> {
        self.active.as_ref().map(|active| active.index)
    }

    pub(super) fn active_running(&self) -> bool {
        self.active_index()
            .is_some_and(|index| self.children[index].running)
    }

    pub(super) fn any_running(&self) -> bool {
        !self.running_jobs.is_empty() || self.children.iter().any(|child| child.running)
    }
}

fn terminal(state: &Value) -> bool {
    matches!(
        state.as_str(),
        Some(
            "done"
                | "complete"
                | "completed"
                | "failed"
                | "error"
                | "stopped"
                | "cancelled"
                | "aborted"
                | "partial"
                | "interrupted"
                | "timed-out"
                | "paused"
                | "timeout"
                | "rejected"
                | "detached"
        )
    )
}

impl ChatView {
    pub(super) fn child_links(
        &self,
        call: Option<&str>,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let call = call?;
        let buttons: Vec<_> = self
            .children
            .children
            .iter()
            .enumerate()
            .filter(|(_, child)| child.key.call == call)
            .map(|(index, _)| self.child_button(index, cx))
            .collect();
        (!buttons.is_empty()).then(|| {
            h_flex()
                .id(SharedString::from(format!("child-links:{call}")))
                .overflow_x_scroll()
                .gap(px(4.))
                .px(px(6.))
                .py(px(4.))
                .children(buttons)
                .into_any_element()
        })
    }

    pub(super) fn child_tabs(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let tabs: Vec<_> = self
            .children
            .children
            .iter()
            .enumerate()
            .filter(|(_, child)| !child.hidden)
            .map(|(index, child)| {
                h_flex()
                    .flex_none()
                    .min_w(px(0.))
                    .max_w_full()
                    .whitespace_nowrap()
                    .child(
                        div()
                            .min_w(px(0.))
                            .overflow_hidden()
                            .child(self.child_button(index, cx)),
                    )
                    .when(!child.running, |tab| {
                        tab.child(
                            IconButton::new(("hide-child", index), IconName::Close)
                                .size(ButtonSize::Compact)
                                .tooltip(Tooltip::text("Hide completed task tab"))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.children.children[index].hidden = true;
                                    cx.notify();
                                })),
                        )
                    })
            })
            .collect();
        (!tabs.is_empty()).then(|| {
            h_flex()
                .id("child-task-tabs")
                .w_full()
                .flex_none()
                .flex_wrap()
                .content_start()
                // 给输入框和消息区留出空间，避免大量标签撑满会话。
                .max_h(px(120.))
                .overflow_y_scroll()
                .gap(px(6.))
                .py(px(4.))
                .children(tabs)
                .into_any_element()
        })
    }

    fn child_button(&self, index: usize, cx: &mut Context<Self>) -> Button {
        let child = &self.children.children[index];
        let label = SharedString::from(self.children.label(index));
        Button::new(("child-task", index), label.clone())
            .tooltip(Tooltip::text(label))
            .icon(if child.running {
                IconName::LoadCircle
            } else {
                IconName::Check
            })
            .style(if self.children.active_index() == Some(index) {
                ButtonStyle::Tinted(TintColor::Accent)
            } else {
                ButtonStyle::Subtle
            })
            .on_click(cx.listener(move |this, _, window, cx| {
                window.focus(&this.focus_handle, cx);
                this.open_child(index, cx);
            }))
    }

    fn open_child(&mut self, index: usize, cx: &mut Context<Self>) {
        if self.children.active_index() == Some(index) {
            return;
        }
        self.children.active = Some(ChildTranscript::new(index));
        self.children.inspection = None;
        self.log_viewer = None;
        self.selection
            .update(cx, |selection, cx| selection.clear(cx));
        self.children.read = Some(cx.spawn(async move |view, cx| {
            loop {
                let Ok((file, parent, is_async)) = view.update(cx, |this, _| {
                    let child = &this.children.children[index];
                    (
                        child.file.clone(),
                        this.session_file.clone(),
                        this.children.jobs.contains_key(&child.key.run),
                    )
                }) else {
                    return;
                };
                let messages = cx
                    .background_spawn(
                        async move { read_record(parent.as_deref()?, file.as_deref()?) },
                    )
                    .await;
                let Ok(running) = view.update(cx, |this, cx| {
                    let Some(active) = this
                        .children
                        .active
                        .as_mut()
                        .filter(|active| active.index == index)
                    else {
                        return false;
                    };
                    let running = this.children.children[index].running;
                    if let Some(messages) = messages {
                        this.children.inspection = None;
                        if active.update(messages, running) {
                            cx.notify();
                        }
                    } else if is_async && this.children.inspection.is_none() {
                        if this.is_running() {
                            return true;
                        }
                        this.inspect_child(index);
                    }
                    running
                }) else {
                    return;
                };
                if !running {
                    return;
                }
                cx.background_executor()
                    .timer(Duration::from_millis(750))
                    .await;
            }
        }));
        cx.notify();
    }

    fn inspect_child(&mut self, index: usize) {
        // 未注册扩展命令时，pi 会把斜线文本作为普通提示发送给模型。
        if self.is_running()
            || !self.slash_commands.iter().any(|command| {
                command.source == slash::Source::Extension
                    && command.name == "subagents-inspect-rpc"
            })
        {
            return;
        }
        let key = &self.children.children[index].key;
        let token = |id: &str| {
            !id.is_empty()
                && id.len() <= 256
                && !id.starts_with('-')
                && !id.contains(char::is_whitespace)
        };
        if !token(&key.run) || !token(&key.child) {
            return;
        }
        self.children.next_inspection += 1;
        let request = format!("gpui-child-{}", self.children.next_inspection);
        let selector = if key.child == key.run { "" } else { &key.child };
        let message = format!(
            "/subagents-inspect-rpc {request} {} {selector} --lines 80",
            key.run
        );
        if let Ok(rpc) = self.send(json!({"type":"prompt", "message":message})) {
            self.children.inspection = Some(PendingInspection {
                child: index,
                request,
                rpc,
            });
        }
    }

    pub(super) fn child_page_open(&self) -> bool {
        self.children.active.is_some()
    }

    pub(super) fn child_page(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let active = self.children.active.as_ref()?;
        let index = active.index;
        let child = &self.children.children[index];
        let colors = &cx.theme().colors;
        let text = |key: &'static str, text: &str| {
            SelectableText::new(
                format!("child-{index}-{key}"),
                text.to_owned(),
                &self.selection,
            )
        };
        Some(v_flex().flex_1().min_h(px(0.))
            .child(h_flex().px(px(12.)).py(px(6.)).gap(px(8.)).bg(colors.panel_background)
                .child(Button::new("close-child-record", "Back to parent").on_click(cx.listener(|this, _, window, cx| {
                    this.children.active = None;
                    this.children.read = None;
                    this.children.inspection = None;
                    this.log_viewer = None;
                    this.selection.update(cx, |selection, cx| selection.clear(cx));
                    window.focus(&this.composer.read(cx).focus_handle(cx), cx);
                    cx.notify();
                })))
                .child(Label::new(self.children.label(index)).truncate()))
            .child(if active.rows.is_empty() {
                v_flex().id(("child-record", index)).flex_1().overflow_y_scroll().gap(px(10.)).p(px(12.))
                    .when(!child.task.is_empty(), |page| page.child(text("task", &child.task)))
                    .child(Label::new(if child.running { "Waiting for readable child history…" } else { "Detailed child history is unavailable. It may have been cleaned up." }).color(Color::Muted))
                    .when(!child.receipt.is_empty(), |page| page.child(text("receipt", &child.receipt)))
                    .into_any_element()
            } else {
                self.render_transcript(&active.list, cx)
            })
            .into_any_element())
    }
}

fn read_record(parent: &Path, file: &Path) -> Option<Vec<Value>> {
    // 日志里的路径不可信，只允许读取对应父会话目录下的子记录。
    let expected = parent.with_extension("");
    let root = expected.canonicalize().ok()?;
    if root
        != parent
            .parent()?
            .canonicalize()
            .ok()?
            .join(parent.file_stem()?)
    {
        return None;
    }
    let file = file.canonicalize().ok()?;
    if !file.starts_with(root) || file.extension()? != "jsonl" {
        return None;
    }
    let metadata = file.metadata().ok()?;
    const MAX_BYTES: u64 = 8 * 1024 * 1024;
    if !metadata.is_file() || metadata.len() > MAX_BYTES {
        return None;
    }
    let mut lines = BufReader::new(std::fs::File::open(file).ok()?.take(MAX_BYTES)).lines();
    if serde_json::from_str::<Value>(&lines.next()?.ok()?).ok()?["type"] != "session" {
        return None;
    }
    let mut entries = HashMap::new();
    let mut leaf = None;
    for line in lines {
        let Ok(entry) = serde_json::from_str::<Value>(&line.ok()?) else {
            continue;
        };
        if let Some(id) = entry["id"].as_str().map(str::to_owned) {
            leaf = Some(id.clone());
            entries.insert(id, entry);
        }
    }
    let mut messages = Vec::new();
    while let Some(mut entry) = leaf.take().and_then(|id| entries.remove(&id)) {
        leaf = entry["parentId"].as_str().map(str::to_owned);
        match entry["type"].as_str().unwrap_or_default() {
            "message" => messages.push(entry["message"].take()),
            "custom_message" => messages.push(json!({
                "role": "custom", "content": entry["content"].take(), "display": entry["display"]
            })),
            "compaction" => messages.push(json!({
                "role": "compactionSummary", "summary": entry["summary"], "tokensBefore": entry["tokensBefore"]
            })),
            "branch_summary" => messages.push(json!({
                "role": "branchSummary", "summary": entry["summary"]
            })),
            _ => {}
        }
    }
    messages.reverse();
    (!messages.is_empty()).then_some(messages)
}

fn inspection_messages(reply: &Value) -> Vec<Value> {
    let mut messages = Vec::new();
    let mut calls = HashMap::<&str, std::collections::VecDeque<usize>>::new();
    for (index, message) in reply["messages"]
        .as_array()
        .into_iter()
        .flatten()
        .enumerate()
    {
        let Some(text) = message["text"].as_str().filter(|text| !text.is_empty()) else {
            continue;
        };
        let name = message["name"].as_str().unwrap_or("tool");
        let content = json!([{"type": "text", "text": text}]);
        match (message["kind"].as_str(), message["role"].as_str()) {
            (Some("text"), Some(role @ ("user" | "assistant"))) => {
                messages.push(json!({"role": role, "content": content}));
            }
            (Some("toolCall"), _) => {
                // ADR-shortcut: 回包缺少调用 ID 时按同名工具的出现顺序配对，协议提供 ID 后改用 ID。
                calls.entry(name).or_default().push_back(index);
                let detail = text
                    .strip_prefix(&format!("[tool: {name} "))
                    .and_then(|text| text.strip_suffix(']'))
                    .unwrap_or(text);
                messages.push(json!({"role": "assistant", "content": [{
                    "type": "toolCall", "id": index.to_string(), "name": name,
                    "arguments": {"command": detail}
                }]}));
            }
            (Some("toolResult"), _) => {
                if let Some(call) = calls.get_mut(name).and_then(|calls| calls.pop_front()) {
                    messages.push(json!({"role": "toolResult", "toolCallId": call.to_string(),
                        "content": content, "isError": message["isError"]}));
                } else {
                    messages.push(json!({"role": "custom", "display": true, "content": content}));
                }
            }
            _ => messages.push(json!({"role": "custom", "display": true, "content": content})),
        }
    }
    if messages.is_empty()
        && let Some(text) = reply["finalOutput"]
            .as_str()
            .filter(|text| !text.is_empty())
    {
        messages.push(json!({"role": "assistant", "content": [{"type": "text", "text": text}]}));
    }
    messages
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queued_background_workflow_does_not_replace_its_eventual_children() {
        let mut children = Children::default();
        children.start("call", &json!({"workflow":"fruit","async":true}));
        children.result(
            "call",
            &json!({"details":{"runId":"job","asyncId":"job"}}),
            ResultPhase::Finished,
        );
        let snapshot = |nodes| json!({"runs":[{"id":"job","kind":"workflow","state":"running","children":nodes}]});
        children.apply_snapshot(&snapshot(json!([])));
        assert!(children.any_running());
        assert!(children.children.is_empty());
        children.apply_snapshot(&snapshot(json!([
            {"id":"apple","kind":"step","label":"worker","state":"running"},
            {"id":"pear","kind":"step","label":"worker","state":"running"}
        ])));
        assert_eq!(children.children.len(), 2);
        assert_eq!(children.children[1].key.child, "pear");
        assert_eq!(children.label(1), "#2 worker · In progress");
    }

    #[test]
    fn workflow_progress_and_final_results_keep_the_same_child_records() {
        let mut children = Children::default();
        children.start("call", &json!({"workflow":"fruit"}));
        let progress = json!({"details":{"runId":"run","workflowChildren":{
            "version":1,"workflowRunId":"run","children":[
                {"childId":"apple","agent":"worker","state":"running","sessionFile":"/tmp/apples.jsonl"},
                {"childId":"pear","agent":"worker","state":"running"}]}}});
        children.result("call", &progress, ResultPhase::Progress);
        assert_eq!(children.children.len(), 2);
        assert!(children.children[0].running);
        let result = json!({"details":{"runId":"run","results":[
            {"workflowKey":"pear","index":0,"agent":"worker","task":"Pears","finalOutput":"Found pears"},
            {"workflowKey":"apple","index":0,"agent":"worker","task":"Apples","finalOutput":"Found apples"}]}});
        children.result("call", &result, ResultPhase::Finished);
        assert_eq!(children.children.len(), 2);
        assert_eq!(children.children[0].receipt.as_ref(), "Found apples");
        assert_eq!(children.children[1].receipt.as_ref(), "Found pears");
        assert!(!children.any_running());
    }

    #[test]
    fn delayed_inspection_reply_cannot_overwrite_another_open_child() {
        let mut children = Children::default();
        children.child("a", "run-a", "run-a", "worker", "Apples");
        children.child("b", "run-b", "run-b", "worker", "Pears");
        children.active = Some(ChildTranscript::new(1));
        children.inspection = Some(PendingInspection {
            child: 0,
            request: "a".into(),
            rpc: "ui-1".into(),
        });
        let reply = json!({"kind":"pi-subagents.inspect-reply","version":1,"requestId":"a","finalOutput":"Apples only"});
        children.inspection_reply(
            &json!({"widgetLines":[format!("PI_SUBAGENT_INSPECT_JSON:{reply}")]}),
        );
        assert!(children.active.unwrap().rows.is_empty());
    }

    #[test]
    fn child_transcript_keeps_tool_details_open_when_results_arrive() {
        let user = json!({"role":"user","content":"Inspect the project"});
        let call = json!({"role":"assistant","content":[
            {"type":"thinking","thinking":"Check the files first"},
            {"type":"toolCall","id":"read-1","name":"read","arguments":{"path":"README.md"}}
        ]});
        let mut transcript = ChildTranscript::new(0);
        assert!(transcript.update(vec![user.clone(), call.clone()], true));
        let Row::Tool {
            status: ToolStatus::Running,
            expanded,
            ..
        } = &mut transcript.rows[2]
        else {
            panic!("expected an active tool call in the third row");
        };
        *expanded = Some(true);
        let result = json!({"role":"toolResult","toolCallId":"read-1",
            "content":[{"type":"text","text":"file output"}],
            "details":{"fullOutputPath":"/tmp/untrusted.log"}});
        let answer = json!({"role":"assistant","content":[{"type":"text","text":"**Read it.**"}]});
        assert!(transcript.update(vec![user, call, result, answer], false));
        assert!(matches!(&transcript.rows[0], Row::User { .. }));
        assert!(
            matches!(&transcript.rows[1], Row::Assistant { thinking, .. }
            if thinking == &["Check the files first"])
        );
        assert!(matches!(&transcript.rows[2], Row::Tool {
            status: ToolStatus::Done, expanded: Some(true), output, log, ..
        } if output == "file output" && log.file.is_none()));
        assert!(
            matches!(&transcript.rows[3], Row::Assistant { text, .. } if text == "**Read it.**")
        );
        assert_eq!(transcript.list.item_count(), 4);
    }

    #[test]
    fn inspection_previews_reuse_user_assistant_and_tool_rows() {
        let reply = json!({"messages":[
            {"role":"user","kind":"text","text":"Check the files"},
            {"role":"assistant","kind":"toolCall","name":"read","text":"[tool: read README.md]"},
            {"role":"toolResult","kind":"toolResult","name":"read","text":"missing file","isError":true},
            {"role":"assistant","kind":"text","text":"**No file.**"}
        ]});
        let rows = history_rows(&inspection_messages(&reply), false);
        assert!(matches!(&rows[..], [Row::User { .. }, Row::Tool {
            status: ToolStatus::Failed, detail, output, ..
        }, Row::Assistant { text, .. }] if detail == "README.md" && output == "missing file" && text == "**No file.**"));
    }

    #[test]
    fn reopening_disambiguates_same_named_workers_and_keeps_each_receipt() {
        let mut messages = vec![];
        for (call, task, output) in [
            ("a", "Find apples", "Found apples"),
            ("b", "Find pears", "Found pears"),
        ] {
            messages.push(json!({"role":"assistant","content":[{"type":"toolCall","id":call,"name":"subagent","arguments":{"agent":"worker","task":task}}]}));
            messages.push(json!({"role":"toolResult","toolCallId":call,"details":{"runId":"r","results":[{"index":0,"agent":"worker","output":output}]}}));
        }
        let mut children = Children::default();
        children.restore(&messages);
        assert_eq!(children.label(0), "#1 worker · Completed");
        assert_eq!(children.label(1), "#2 worker · Completed");
        assert_eq!(children.children[0].task, "Find apples");
        assert_eq!(children.children[1].task, "Find pears");
        assert_eq!(children.children[0].receipt.as_ref(), "Found apples");
        assert_eq!(children.children[1].receipt.as_ref(), "Found pears");
        assert!(
            children
                .children
                .iter()
                .all(|child| child.hidden && !child.running)
        );
    }
}
