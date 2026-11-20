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

#[derive(Default)]
pub(super) struct Children {
    calls: HashMap<String, Value>,
    jobs: HashMap<String, String>,
    running_jobs: std::collections::HashSet<String>,
    snapshot: Option<Value>,
    inspection: Option<PendingInspection>,
    next_inspection: u64,
    children: Vec<Child>,
    active: Option<usize>,
    detail: Option<String>,
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
        if self.active != Some(inspection.child) {
            return;
        }
        let child = &mut self.children[inspection.child];
        if let Some(task) = reply["task"].as_str()
            && child.task.is_empty()
        {
            child.task = task.into();
        }
        if terminal(&reply["status"]) {
            child.running = false;
        }
        let text = reply["messages"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|message| {
                let text = message["text"].as_str()?;
                Some(format!(
                    "{}\n{text}",
                    message["role"].as_str().unwrap_or("message")
                ))
            })
            .collect::<Vec<_>>()
            .join("\n\n");
        self.detail = if text.is_empty() {
            reply["finalOutput"]
                .as_str()
                .filter(|text| !text.is_empty())
                .map(str::to_owned)
        } else {
            Some(text)
        };
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
        let title = format!("#{} {}", index + 1, child.name);
        if child.task.is_empty() {
            title
        } else {
            format!("{title} · {}", sessions::one_line(&child.task, 46))
        }
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
                    .child(self.child_button(index, cx))
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
                .overflow_x_scroll()
                .gap(px(6.))
                .px(px(10.))
                .py(px(4.))
                .border_t_1()
                .border_color(cx.theme().colors.border)
                .children(tabs)
                .into_any_element()
        })
    }

    fn child_button(&self, index: usize, cx: &mut Context<Self>) -> Button {
        let child = &self.children.children[index];
        Button::new(("child-task", index), self.children.label(index))
            .icon(if child.running {
                IconName::LoadCircle
            } else {
                IconName::Check
            })
            .style(if self.children.active == Some(index) {
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
        if self.children.active == Some(index) {
            return;
        }
        self.children.active = Some(index);
        self.children.detail = None;
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
                let detail = cx
                    .background_spawn(
                        async move { read_record(parent.as_deref()?, file.as_deref()?) },
                    )
                    .await;
                let Ok(running) = view.update(cx, |this, cx| {
                    if detail.is_some() || !is_async {
                        if this.children.detail != detail {
                            this.children.detail = detail;
                            cx.notify();
                        }
                    } else {
                        this.inspect_child(index);
                    }
                    this.children.children[index].running
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
        let index = self.children.active?;
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
                    this.selection.update(cx, |selection, cx| selection.clear(cx));
                    window.focus(&this.composer.read(cx).focus_handle(cx), cx);
                    cx.notify();
                })))
                .child(Label::new(self.children.label(index)).truncate()))
            .child(v_flex().id(("child-record", index)).flex_1().overflow_y_scroll().gap(px(10.)).p(px(12.))
                .when(!child.task.is_empty(), |page| page.child(text("task", &child.task)))
                .when_some(self.children.detail.as_deref(), |page, detail| page.child(text("detail", detail)))
                .when(self.children.detail.is_none(), |page| page
                    .child(Label::new(if child.running { "Waiting for readable child history…" } else { "Detailed child history is unavailable. It may have been cleaned up." }).color(Color::Muted))
                    .when(!child.receipt.is_empty(), |page| page.child(text("receipt", &child.receipt)))))
            .into_any_element())
    }
}

fn read_record(parent: &Path, file: &Path) -> Option<String> {
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
    while let Some(entry) = leaf.take().and_then(|id| entries.remove(&id)) {
        leaf = entry["parentId"].as_str().map(str::to_owned);
        if entry["type"] == "message" {
            let message = &entry["message"];
            let text = sessions::message_text(message);
            if !text.is_empty() {
                messages.push(format!(
                    "{}\n{text}",
                    message["role"].as_str().unwrap_or("message")
                ));
            }
        }
    }
    messages.reverse();
    (!messages.is_empty()).then(|| messages.join("\n\n"))
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
        children.active = Some(1);
        children.detail = Some("Pears only".into());
        children.inspection = Some(PendingInspection {
            child: 0,
            request: "a".into(),
            rpc: "ui-1".into(),
        });
        let reply = json!({"kind":"pi-subagents.inspect-reply","version":1,"requestId":"a","finalOutput":"Apples only"});
        children.inspection_reply(
            &json!({"widgetLines":[format!("PI_SUBAGENT_INSPECT_JSON:{reply}")]}),
        );
        assert_eq!(children.detail.as_deref(), Some("Pears only"));
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
        assert_eq!(children.label(0), "#1 worker · Find apples");
        assert_eq!(children.label(1), "#2 worker · Find pears");
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
