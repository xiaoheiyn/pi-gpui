# 本机 pi-subagents 怎样标识调用并提供子会话记录

**同名并发时，先用父会话内的 `toolCallId` 找到发起调用，再用扩展返回的标识找到子记录。** 普通前台运行使用 `(runId, index)`。后台检查使用 `(asyncId, childId)`，其中 `childId` 取自该运行的状态快照。重开父会话后，父工具结果和完成通知能说明运行过什么，子会话文件或仍保留的运行记录决定能展开多少历史。[扩展关联约定][api] · [后台检查协议][inspect]

本次对照本机 npm 包 `pi-subagents@0.89.1` 的编译产物、公开标签 `v0.89.1`（npm 的 `gitHead` 与标签解引用均为 `533033f`）、Pi `1.5.2` 和本仓库 `a8d40e3`。以下示例标识均为合成值。[npm 发布记录][npm] · [Pi RPC 源码][pi-rpc] · [仓库聊天模型][chat]

## 同时启动两个 worker，如何归属

| 已知对象 | 可用于关联的证据 | 同名示例与边界 |
| --- | --- | --- |
| 父会话内的一次工具调用 | 实时事件的 `toolCallId`。历史中 assistant 的 `content[].type="toolCall"` 块的 `id` 对应 `role="toolResult"` 消息的 `toolCallId`。 | 两次 `subagent` 都调用 `worker`，按 `call-a / call-b` 分行。两次查询同一运行的 `status` 也各有自己的调用行。[仓库事件和历史配对][chat] · [上游对重复查询的说明][obs] |
| 同一前台运行中的子任务 | `details.runId` 加每行明确的 `details.results[].index`。索引按启动分配，从部分进度到终态保持稳定。 | `(run-a, 0)`、`(run-a, 1)` 可以同叫 `worker`；另一运行可以再用索引 0。数组位置、完成顺序、角色名和 `sessionName` 都会误配。[字段定义][types] · [进度约定][api] |
| 后台运行及工作流 | 直接后台启动回执通常有 `details.asyncId`、`runId`、`asyncDir`，`results` 可以为空。选子节点时原样传快照的节点 ID，可能是 `workflowKey`、子 `runId`、`step:<index>` 或嵌套运行 ID。工作流子状态还有 `(parentWorkflowRunId, workflowKey)`。 | 同名 `worker` 可分属键 `left / right`。工作流子状态里的 `toolCallId` 由工作流宿主生成。恢复执行可能产生新 `runId` 并复用原 `sessionFile`。状态中的 `sessionId` 通常表示所属父会话文件，未持久化时可为父会话 ID。[回执源码][async] · [运行字段][obs] · [恢复语义][types] · [归属字段][lifetime] |

进程内扩展 RPC 的直接异步 `spawn` 若丢了应答，可让 `status` 查询 `rpc-spawn-<原 requestId>`。旧记录可能缺 `toolCallId`，同一别名也可能命中多个运行。缺证不能推出“从未启动”，不能据此自动重发。Fleet 的展示 `key` 是不透明值。[直接异步调用关联][api]

## 运行中能看到什么

**前台。** 父进程收到 `tool_execution_start / update / end`。对同一 `toolCallId`，更新读 `partialResult.details`，结束读 `result.details`。前台子会话直接在父 Pi 进程中运行，进度行有 `index`、`agent`、`recentTools`、`recentOutput` 等摘要。有子会话文件时，启动进度 `details.progress[].sessionFile` 就给出位置，路径可能先于第一条落盘消息出现。父工具卡片的更新与子消息落盘节奏可以不同。[前台启动源码][fg] · [进度约定][api] · [仓库事件处理][chat]

**后台。** 父工具结束可只是交回启动回执。RPC 宿主从 `extension_ui_request / setWidget` 的 `subagent-async` 小组件接收 `PI_SUBAGENT_ASYNC_JSON:` 有界状态快照。查看特定孩子时，在同一父 RPC 会话用 `prompt` 发送扩展命令，例如 `/subagents-inspect-rpc req-a run-a step:1 --lines 40`（仅当快照给出 `step:1`）。它直接执行，无需模型回合。实际数据在 `subagent-inspect` 小组件的一行 `PI_SUBAGENT_INSPECT_JSON:` 中，发出后即撤回。按内层 `requestId` 收取 `pi-subagents.inspect-reply` v1；两次检查可有不同请求 ID 却检查同一孩子。[后台检查协议][inspect] · [Pi 命令][commands] · [Pi 小组件协议][widget]

检查结果可含 `status`、`messages`、`task`、`finalOutput`，消息窗口上限 200、序列化上限 64 KiB。运行中可能只有已有消息。分叉子会话带有父历史，检查器只在可归属的全窗口 fresh 会话中取首条用户消息作任务。进程内 `pi.events` 的生命周期提示不跨 Pi 进程，重连也不回放；桌面端的 stdio RPC 可用上面的小组件协议。[检查实现与测试][inspect-src] · [扩展事件作用域][api]

## 完成与重开

1. **先重建父调用。** 本仓库每个聊天有自己的 `pi --mode rpc`，重开时带 `--session`，再用 `get_messages` 取得该会话当前的消息。前台从配对的 `subagent` 工具结果取 `details.results[]`。终态压缩会移除内联 `messages` 和逐步 `progress`，保留可用的工具摘要、文件引用等字段。后台从 `role="custom", customType="subagent-notify"` 的 `details.runs[]` 按 `runId` 对回执；合并通知可含多个运行，`childRuns[]` 可给子运行 ID 和键。[进程启动][launch] · [Pi RPC 实现][pi-rpc] · [终态压缩][compact] · [通知字段][lifetime]
2. **再展开所选孩子。** `sessionFile` 是 Pi 子会话 JSONL，可含用户消息、助手的工具调用和工具结果。Pi 的会话条目以 `id / parentId` 构成树；要还原某一分支，需沿该分支祖先关系读取。父会话的 `get_entries` 返回父文件的全部条目及 `leafId`，不会递归返回子文件。后台运行时路径可能在 `steps[i].sessionFile`，单子运行完成后才有顶层路径。只使用当前会话、该运行实际记录且通过可信根校验的引用。[Pi 会话格式][session] · [Pi RPC 实现][pi-rpc] · [路径选择与校验][inspect-src] · [后台状态字段][obs]
3. **辨认记录种类。** `transcriptPath` 或 `artifactPaths.transcriptPath` 可指向扩展自己的事件 JSONL。其行带 `version / recordType / runId` 及可选 `childIndex`，子工具事件还有自己的 `toolCallId`；写入器会标记截断或报告错误。异步目录里的 `status.json` 是状态，`events.jsonl` 记录生命周期并省略 `message_update`，`output-<n>.log` 用于看输出尾部。只剩最终输出文件时，只能展示最终输出。[事件记录写入器][writer] · [运行产物说明][obs]

**现有桌面端的边界。** `src/chat.rs` 已按调用 ID 更新和恢复工具行，但主要留下文本、图片和 `details.fullOutputPath`；可见 custom 消息重建为普通通知，`setWidget` 分支当前忽略。未配到工具结果的父调用重开后显示失败，此状态不足以判断后台孩子是否启动。`src/sessions.rs` 按工作区筛选会话标题，也不提供父调用到孩子的映射。[仓库聊天模型][chat] · [会话枚举][sessions-rs]

## 记录缺失时的结论

| 观察 | 可向读者表达的结论 |
| --- | --- |
| 已收到前台运行标识，文件尚未出现；或后台检查成功但缺 `messages` | “已开始，暂无可读的子消息”。保留父行和已知进度。可选字段缺失不足以判断空会话。[进度约定][api] · [检查实现][inspect-src] |
| `not_found`、`stale`、`foreign_session` | 依次表示“找不到该异步运行或子节点”“找到了运行线索，但所需状态已清理”“当前会话无权查看”。线索全失也可能是 `not_found`。格式损坏可能返回 `internal`；不应借同名兄弟补齐。[检查实现][inspect-src] · [上游归属及旧版同名测试][test-inspect] |
| 父通知仍在，一次性结果或子文件已删除 | 运行结束后，结果文件在通知送达后消费删除，扩展会先写临时 completion replay 与有界输出归档。若归档可读则可展示其覆盖的部分，否则保留父回执并注明“子历史不可用”。会话类制品默认清理期为 7 天，可配置；replay 另有去重保留窗口，两者均不能作为永久历史承诺。[消费与回放][obs] · [清理配置][retention] |

## 核验方法

在隔离的临时目录运行固定上游提交的 `inspect-rpc.test.ts` 与 `child-transcript.test.ts`，仅把测试导入后缀从上游 TypeScript 改为本机 npm 包编译的 JavaScript。使用 `node --experimental-strip-types --test` 得到 **44 项通过，0 项失败**。用例覆盖运行中的消息、完成后从 replay 取回指定孩子的输出、旧记录中同名子任务的输出归属、无权访问和未知 ID。前台索引和桌面重开路径按对应版本源码核对。全部输入取自发布代码、文档和合成夹具。[检查测试][test-inspect] · [记录写入测试][test-transcript]

[npm]: https://registry.npmjs.org/pi-subagents/0.89.1
[api]: https://github.com/nicobailon/pi-subagents/blob/533033f0e906e85fdb0e909ce27a7c499ffb6485/docs/extension-api.md#refresh-a-host-view-without-polling
[inspect]: https://github.com/nicobailon/pi-subagents/blob/533033f0e906e85fdb0e909ce27a7c499ffb6485/docs/observability.md#host-inspection-protocol-rpc
[obs]: https://github.com/nicobailon/pi-subagents/blob/533033f0e906e85fdb0e909ce27a7c499ffb6485/docs/observability.md
[types]: https://github.com/nicobailon/pi-subagents/blob/533033f0e906e85fdb0e909ce27a7c499ffb6485/src/shared/types.ts
[async]: https://github.com/nicobailon/pi-subagents/blob/533033f0e906e85fdb0e909ce27a7c499ffb6485/src/runs/background/async-execution.ts
[lifetime]: https://github.com/nicobailon/pi-subagents/blob/533033f0e906e85fdb0e909ce27a7c499ffb6485/docs/extension-api.md#host-session-lifetime-and-completion-wakes
[fg]: https://github.com/nicobailon/pi-subagents/blob/533033f0e906e85fdb0e909ce27a7c499ffb6485/src/runs/foreground/execution.ts
[commands]: https://github.com/earendil-works/pi/blob/b4afdddf44e059c67ccf5097e5317d913521b470/packages/coding-agent/docs/rpc-commands.md
[widget]: https://github.com/earendil-works/pi/blob/b4afdddf44e059c67ccf5097e5317d913521b470/packages/coding-agent/docs/rpc-extension-ui.md#setwidget
[inspect-src]: https://github.com/nicobailon/pi-subagents/blob/533033f0e906e85fdb0e909ce27a7c499ffb6485/src/runs/background/inspect-rpc.ts
[compact]: https://github.com/nicobailon/pi-subagents/blob/533033f0e906e85fdb0e909ce27a7c499ffb6485/src/shared/utils.ts
[session]: https://github.com/earendil-works/pi/blob/b4afdddf44e059c67ccf5097e5317d913521b470/packages/coding-agent/docs/session-format.md
[writer]: https://github.com/nicobailon/pi-subagents/blob/533033f0e906e85fdb0e909ce27a7c499ffb6485/src/shared/child-transcript.ts
[retention]: https://github.com/nicobailon/pi-subagents/blob/533033f0e906e85fdb0e909ce27a7c499ffb6485/docs/configuration.md#artifactdir
[test-inspect]: https://github.com/nicobailon/pi-subagents/blob/533033f0e906e85fdb0e909ce27a7c499ffb6485/test/unit/inspect-rpc.test.ts
[test-transcript]: https://github.com/nicobailon/pi-subagents/blob/533033f0e906e85fdb0e909ce27a7c499ffb6485/test/unit/child-transcript.test.ts
[pi-rpc]: https://github.com/earendil-works/pi/blob/b4afdddf44e059c67ccf5097e5317d913521b470/packages/coding-agent/src/modes/rpc/rpc-mode.ts
[chat]: ../../../src/chat.rs
[launch]: ../../../src/pi.rs
[sessions-rs]: ../../../src/sessions.rs
