# Qt 桌面应用自动化测试智能体

在 `cua-driver`（Rust）之上扩展的 Windows 桌面 UI 测试内核：用 YAML 写用例，用稳定身份定位 Qt 控件，用结构化效果与断言判定通过/失败，并产出可追溯的证据与报告。

> 状态：**M1 骨架已落地**（2026-11-05）。Rust 1.97.1 已构建通过，`cua-driver-qt` crate 注册了 `qt_resolve`，并在 Calculator / Qt Creator 上完成唯一命中、身份跨观测稳定、歧义拒绝、未找到、截断走查、最小化拒绝与选择器错误分类验证。测试中发现 Qt 自绘按钮（无 UIA Pattern）会被整条丢弃，已改为"可观测 ≠ 可操作"纳入观测树。尚待：Qt 桥决策（Q1）与 30 控件样板应用。

## 1. 解决什么问题

现有 runtime 能"看见并操作"窗口，但只能回答"点击有没有发出去"，回答不了测试真正关心的问题：
1. **这是不是同一个控件？** `element_index` 只在上一次观测内有效，跨步骤就失效。
2. **操作有没有生效？** 事件送达不等于业务状态变化。
3. **这次运行为什么失败？** 没有选择器打分、动作效果、截图与树的证据链。

本项目补三层：**身份层**（选择器 + 指纹）、**判定层**（动作效果 + 三态断言）、**编排层**（用例 + 报告）。

## 2. 不是什么

| 不做 | 原因 |
| --- | --- |
| 通用 RPA / 办公自动化 | 只服务桌面应用测试 |
| 替代 QTest / 单元测试 | 进程外黑盒，与进程内测试互补 |
| 以 OCR / 模板匹配为主要识别手段 | 像素只做坐标与视觉回归 |
| 首期支持移动端 / Web / Electron | 架构留 Adapter，不在首期实现 |
| 录制回放 | 列为后续能力 |

## 3. 核心结论

| 项目 | 结论 |
| --- | --- |
| 难度 | 6/10（中等偏上）：上游已提供令牌、`verify_state`、输入与捕获，难点是坐标归一化、Qt 结构缺失、非确定性 |
| 基线 | `cua` → `libs/cua-driver/rust`（MIT）；复用其工具注册表、快照令牌、三态校验与 Windows 平台层 |
| 架构 | 三平面（感知执行 / 归一化 / 测试语义）+ 六个扩展注册表 |
| 工期 | 100 人日；两人并行 9–11 周；应用团队另需 13–25 人日 |
| 第一件事 | ~~装 Rust 并构建 `cua-driver`~~（已完成）；下一步是 Q1：被测应用能否加 `--test-mode` 测试桥，以及 30 控件样板应用 |

## 4. 文档

| 文档 | 内容 | 读者 |
| --- | --- | --- |
| [需求文档](./REQUIREMENTS.md) | 目标、缺口、功能需求 FR-01..FR-13、验收标准、里程碑 | 产品 / 测试 / 评审 |
| [详细设计](./DESIGN.md) | 难度判断、ADR 选型、三平面架构、模块与契约、扩展点、工作量、风险 | 开发 / 架构 |
| [cua-driver 说明](../../README.md) | 基线项目的能力边界、安装与运行方式 | 所有实现者 |
| [cua-driver 文档目录](../) | 基线项目的设计文档与测试指南 | 所有人 |

阅读顺序：先读本页 → 需求文档第 1–3 节 → 设计文档第 1–3 节 → 按任务深入。

## 5. 关键决策（先看这几条）

| 编号 | 决策 | 一句话理由 |
| --- | --- | --- |
| ADR-01 | 扩展 `cua-driver`（Rust），新增 `cua-driver-qt` crate | 上游已有令牌、`verify_state`、后台输入与工具注册表；缺的正好是身份/判定/编排 |
| ADR-02 | 复用上游既有 runtime，不新建守护进程 | MCP 可 `--direct` 进程内运行；CLI `call` 走本地 `serve` daemon；两者共用 `register_host_tools` 注册钩子 |
| ADR-05 | 身份与句柄强制分离 | 句柄是观测内编号，身份是跨运行指纹；混用必然 flake |
| ADR-06 | 复用 `snapshot_id` + `element_token` + `invalidated_snapshot_ids`，增量补 `revision` | 令牌失效上游已实现，自建等于重写；`revision` 只补排序 |
| ADR-07 | 动作 `effect` 与断言 `verification` 分离 | "点了"不等于"生效了"，更不等于"业务通过" |
| ADR-08 | 协议三层：单动作 / `qt_run_steps` / `qt_run_case` | 观测 0.5–3 s，必须允许批处理 |
| ADR-11 | Qt 桥是应用内可选插件，不做注入 | 注入会被安全软件拦截且责任不清 |
| ADR-13 | Rust 编译期注册 + 稳定 JSON 契约 | 单一注册点，上游 rebase 冲突面最小 |
| ADR-14 | 内核与框架无关，Qt 只是 profile | 否则接第二个框架就要返工 |

完整 14 条见[详细设计](./DESIGN.md)第 2 节。

## 6. 前置条件

| 编号 | 问题 | 不解决的后果 |
| --- | --- | --- |
| Q1 | 应用能否加 `--test-mode` 测试桥？ | 自绘控件与 3D 视口只能坐标 + 像素断言 |
| Q2 | Qt 版本与 QML 占比？ | 决定桥的实现方式与验收样例 |
| Q3 | 3D 视口能否提供只读状态 API？ | 视觉断言成为唯一手段 |
| Q4 | CI 是否有交互式桌面会话？ | 流水线里 UIA 树为空，无法跑 UI 测试 |
| Q5 | 是否需要 macOS 同步支持？ | 决定跨平台工作量 |

## 7. 预期的使用形态

```bash
# CLI 的 call 走本地 daemon（另开一个终端先起服务）
cua-driver serve

# 解析选择器，打印候选与打分（不执行任何动作）
# M1 已实现 uia(...)；widget(...) 待 Qt 桥
cua-driver call qt_resolve '{"pid":1234,"window_id":56789,"selector":"uia(name=\"开始扫描\", controlType=\"Button\")"}'

# 跑一条 YAML 用例，产出 JUnit 与证据目录
cua-driver call qt_run_case '{"case":"cases/scan-smoke.yaml","keep_evidence":"on_failure"}'

# MCP：Agent 通过技能包调用
qt_resolve / qt_act / qt_assert / qt_run_case
```

上游既有工具的名称、参数与返回结构保持不变；新能力走 `qt_*` 命名空间。

## 8. 预计代码落点

```text
libs/cua-driver/
  rust/
    crates/cua-driver-qt/      # Rust 测试内核（模型、选择器、身份、动作、断言、runner、报告）
    crates/platform-windows/   # UIA 走查与输入；内容节点矩形由本项目增量补充
    crates/cua-driver/src/main.rs
                               # register_host_tools 末尾加一行注册
  bridges/qt-test-bridge/      # C++ 可选插件（CMake / qmake 接入示例）
  skills/qt-test-agent/        # Agent 技能包
  docs/qt-test-agent/          # 本文档（与 crate 同仓库）
```

依赖方向：`tools → runner/selector/action/assert → adapter → model`；`model` 不依赖其他模块。

## 9. 许可与依赖

- 基线 `cua-driver` 为 MIT，可直接修改与分发；本项目新增代码沿用 MIT。
- 不引入 `cua-perception`（OmniParser，AGPL-3.0-only）与 `cua-som`（AGPL-3.0-or-later）；Cua Spaces / Driver Bench（FSL-1.1-MIT）同样不引入。
- Qt 桥由被测应用自行编译加载，本项目只提供接口与示例。
