# Qt 桌面应用自动化测试智能体 — 详细设计

| 项目 | 内容 |
| --- | --- |
| 文档版本 | v0.3（草案，待评审） |
| 日期 | 2026-11-05 |
| 基线项目 | `cua` → `libs/cua-driver/rust`（MIT） |
| 上游文档 | [需求文档](./REQUIREMENTS.md) v0.3 |
| 目标平台 | Windows 优先；macOS / Linux 只保证接口不阻塞 |
| 设计范围 | 通用测试内核 + Qt 接入配置 + 二次开发扩展点 |

> **v0.3 变更要点**：基线从 `open-computer-use`（Go）切换为 `cua-driver`（Rust）。切换理由是 cua 已经具备本项目最难的三个基础件——快照令牌与失效（`snapshot_id` + `element_token` + `invalidated_snapshot_ids`）、三态状态校验（`verify_state`）、后台优先输入与窗口级捕获。我们只补它没有的部分：Qt 身份层、选择器解析、用例编排与报告。

---

## 1. 结论：难度判断

### 1.1 总判断

**中等偏上（6/10），单人 2.5–3.5 人月可到可用 v1。**

不是"再造一个 computer-use"。感知与执行那一层已经存在，而且比原先评估的更完整：`cua-driver` 已有窗口级捕获、后台优先输入、快照令牌失效、元素令牌（`element_token`）、三态 `verify_state`、`elements_complete` 与 PID/window 守卫。新增的是三层：

1. **身份层**——把"一次观测里的第 7 个节点"变成"这个按钮"。
2. **判定层**——把"点了"变成"生效了 / 没生效 / 不知道"。
3. **编排层**——把"一串调用"变成"一条用例、一份报告"。

代码量不大（新增约 9–13k 行 Rust + 1k 行 C++），难点集中在三个地方：

| 难点 | 为什么难 | 影响 |
| --- | --- | --- |
| 跨通道坐标归一化 | UIA 返回物理像素，WGC 返回截图像素，Qt 返回逻辑像素，DPI 缩放下三者不一致 | 点偏、断言区域偏，且错得很隐蔽 |
| Qt 结构缺失 | 自绘控件、`QOpenGLWidget`、部分 QML 在 UIA 里只有一个顶层节点 | 选择器无解，只能退坐标 |
| 非确定性 | 动画、异步加载、GPU 抗锯齿、字体渲染 | 20 次跑 19 次过，剩下 1 次查一天 |

### 1.2 分层难度

| 层 | 内容 | 难度 | 主要风险 |
| --- | --- | --- | --- |
| L0 | 复用 Windows runtime、接好工具注册与 CLI/MCP | 3/10 | 读懂 `cua-driver` 的 Tool/Registry 边界与平台 crate 分层；Rust 工具链首装 |
| L1 | 归一化节点模型 + 多通道适配 | 5/10 | 坐标空间统一，字段对齐 |
| L2 | 选择器语法 + 匹配打分 | 4/10 | 歧义策略（宁可失败不可误点） |
| L3 | 身份指纹 + 令牌失效 | 5/10 | 动态内容导致的指纹漂移 |
| L4 | 动作效果分级 + invoke/setValue | 5/10 | 读回验证的设备差异 |
| L5 | Qt 测试桥（C++ 插件） | 7/10 | 线程亲和、构建接入、生产剥离、安全软件 |
| L6 | 断言 + 用例 + 报告 | 3/10 | 重试策略设计 |
| L7 | 视觉回归 | 5/10 | 基线稳定性，不是算法难 |
| L8 | 权限与安全 | 3/10 | 复用现有门禁即可，别自建策略引擎 |
| L9 | 技能包 | 2/10 | 文档质量 |

### 1.3 难度会在什么条件下变化

**升到 8/10：**

- 要求完全无侵入，同时覆盖自绘 3D 视口和全部 QML 控件；
- 要求 Windows / macOS / Linux 同步交付；
- 要求录制回放、自然语言生成用例；
- 要求在没有交互式桌面会话的 CI 容器里跑。

**降到 4/10：**

- 应用团队愿意加 `objectName` / `setAccessibleName` 和 `--test-mode`；
- 首期只覆盖 QWidget 标准控件 + 少量 QML；
- 只做 Windows。

**判断依据：** Qt 桥是唯一必须动被测应用代码的组件。它决定的是"能不能看见"，不是"代码写不写得出来"。所以项目启动的第一件事不是写代码，是确认 Q1（应用是否允许加测试桥）。

---

## 2. 选型决策（ADR）

每个决策记录：选了什么、放弃了什么、为什么、可逆性。

| 编号 | 决策 | 选择 | 放弃的方案 | 理由 | 可逆性 |
| --- | --- | --- | --- | --- | --- |
| ADR-01 | 基线仓库 | 扩展 `cua` 的 `libs/cua-driver/rust` | 自建；扩展 `open-computer-use`；直接用 Qoder/Codex | 两者都是 MIT；`cua-driver` 已提供快照令牌、`verify_state`、`elements_complete`、后台输入、窗口级捕获与工具注册表，缺的正好是身份/判定/编排三层；`open-computer-use` 的 Windows runtime 更薄，这些都要自己写 | 中（新增工具与上游解耦，可随时脱离） |
| ADR-02 | 驱动形态 | 复用 `cua-driver` 既有 runtime，新增 crate 注册工具 | 新建守护进程；自建 UniFFI 绑定；Go 单 exe | MCP 可用 `--direct` 在进程内拥有 runtime；CLI `call` 则要求本地 `serve` daemon 在监听（上游实现如此）。两条入口都经同一个 `register_host_tools` 钩子，qt 工具自动可见；不必为新增层再起一个进程 | 高 |
| ADR-03 | 观测模型 | 统一 `Node` 模型 + 多 Adapter | 每个框架一套模型 | 选择器、断言、报告只写一次；新增框架只写 Adapter | 高 |
| ADR-04 | 通道优先级 | Qt 桥 > UIA > 像素 | UIA 优先 | Qt 桥能拿到 `objectName` 和全部 QObject；UIA 对自绘缺陷明显 | 高 |
| ADR-05 | 身份与句柄 | 强制分离 | 跨步骤复用 `element_index` | 句柄是观测内编号，身份是跨运行指纹；混用必然 flake | 低（改契约） |
| ADR-06 | 令牌语义 | 复用 `snapshot_id` + `element_token` + `invalidated_snapshot_ids`，增量补 `revision` | 自建双令牌体系；仅返回序号 | 上游已实现令牌失效与 `since` 差分读取，自建等于重写；`revision` 只作为同一个 `snapshot_id` 之外的单调序号，用于报告排序与跨通道对齐 | 低 |
| ADR-07 | 动作结果 | `effect` 与 `verification` 分离 | 只返回 true/false | "输入已投递"不等于"界面已生效"；未知不能当成功 | 低 |
| ADR-08 | 协议粒度 | 三层：单动作 / `qt_run_steps` / `qt_run_case` | 只做单动作；只做高层 | Agent 探索要单动作，CI 要高层；实测观测 0.5–3 s，必须允许批处理 | 高 |
| ADR-09 | 断言表达 | 内置 `kind` 枚举 + 参数 | 通用表达式语言；脚本断言 | v1 不做语言，避免权限与可读性问题；复杂逻辑走 `run_script` | 高 |
| ADR-10 | 权限 | 3 级风险 + `--test-mode` 门控 + 动作时确认 | 移植 cua 的 7 层策略 + Rego | 测试场景明确，7 层是给多租户网关用的；过度设计 | 高 |
| ADR-11 | Qt 桥形态 | 应用内可选插件（命名管道 JSON-RPC） | 远程注入 DLL；驱动级事件 | 注入会被安全软件拦截且不可控；插件由应用自己加载，责任边界清晰 | 中 |
| ADR-12 | 视觉回归 | ROI + 掩膜 + SSIM | 全图逐像素 | 全图会被光标、时间戳、动画帧干扰，误报率高 | 高 |
| ADR-13 | 二次开发机制 | Rust 编译期注册（`ToolRegistry::register`）+ 稳定 JSON 契约 | 动态插件（`libloading`）；进程外插件 | 宿主已有 `DriverHostOptions::register_host_tools` 单一钩子；v1 用注册表足够，JSON 契约保证外部工具可读；新工具先走 runtime schema，通过后再进 contract | 高 |
| ADR-14 | 通用性边界 | 内核与框架无关，Qt 作为 profile | 内核里写死 Qt | "可通用使用"要求同一个内核能接 WPF、Win32、Electron；Qt 只是第一个 profile | 高 |

### 2.1 为什么选 cua-driver，以及只抄哪些

`cua` 的文档质量是这批实现里最高的，而且它的实现已经覆盖了本项目最难的基础件。选择它是复用，不是崇拜。

**直接复用（不改上游语义）：**

| 能力 | 位置 | 本项目怎么用 |
| --- | --- | --- |
| 工具注册与 MCP/CLI 双入口 | `cua-driver-core::tool::{Tool, ToolRegistry}`、`cua-driver` 的 `DriverHostOptions::register_host_tools` | 新增 crate 提供 `register_qt_tools(&mut ToolRegistry)`，在宿主注册钩子 `cua-driver/src/main.rs::register_host_tools` 追加一行；CLI `call`、MCP、`tools` 元数据三条路径共用 |
| 快照令牌与失效 | `element_token`、`get_window_state` 的 `snapshot_id` / `element_token` / `invalidated_snapshot_ids` / `since` | 选择器解析结果直接携带令牌；动作层复用既有过期拒绝 |
| 三态状态校验 | `cua-driver-core::expectation::VerifyStateTool`（`VerificationStatus` / `UnknownReason`） | 复用其轮询、稳定采样与三态语义。注意上游 `ElementSelector` 只支持 `role` + `label_contains`，表达不了 Qt 选择器；因此 `qt_assert` 在归一化观测上判定，仅在可等价映射时委托 `verify_state` |
| 完整性标记 | `elements_complete` | `notExists` 不能由上游 `exists:false` 证明（会被拒绝）；只有 `elements_complete=true` 时才允许本地判定"确实不存在" |
| 后台优先输入与窗口级捕获 | `platform-windows::{input,capture}`、`pid_window_guarded` | 动作阶梯的底层两跳；PID/窗口守卫直接继承 |
| 工具契约 | `cua-driver-contract`（`ToolContract`、`schema_mode`） | 新工具先在 runtime schema 稳定，再迁入 contract；不阻塞首期 |

**明确不引入：**

- **`cua-perception` / `cua-som`（AGPL-3.0）**：本项目的重点是可定位元素，不是图标识别；不安装、不分发、不链接。许可证不是障碍，但引入它会把 AGPL 传染风险带进制品的审计范围。
- **Cua Spaces / Driver Bench（FSL-1.1-MIT）**：与测试执行无关。
- **7 层授权策略引擎**：测试机是单租户，保留上游既有的权限门禁即可，不新增 YAML/Rego 策略层。
- **UniFFI Python/TS 绑定**：首期只需要 Rust 内核 + CLI/MCP；绑定在 contract 稳定后再生成，避免每次改 schema 都要双端重编。

**上游修改策略（关键）：**

本项目以"**新增 crate + 一行注册**"的方式接入，不改 `Tool` trait、不改既有工具签名、不改 `get_window_state` 的必填字段。允许的改动只有五类：

1. 新增 workspace crate（`crates/cua-driver-qt`）；
2. 在宿主注册钩子 `cua-driver/src/main.rs::register_host_tools` 追加一行 `cua_driver_qt::register_qt_tools(registry);`；
3. 在 `cua-driver-core::authorization` 的风险分类表登记新工具（只读观测按 `get_window_state` 同级归 R3），否则授权层对未登记工具 fail-closed 拒绝；
4. `platform-windows::uia::emit_cached_node` 为"有内容但无可缓存 Pattern"的节点也读取包围矩形（原先这类节点 `rect = None`、`center = 0`）。原因：Qt 的 UIA 桥不给自绘按钮暴露 Invoke，Qt Creator 的"运行""开始调试"就属于这类；不读矩形，身份层看不到它们，也做不了坐标与越界检查。改动只是把已有的 `read_cached_bounding_rect_full` 调用提到分支之前，markdown 与既有字段不变；
5. 对既有输出做**纯增量字段**（例如 `revision`），旧调用方忽略即可。

**为什么不是 `platform-windows::tools::impl_`：** `cua-driver-qt` 要复用 `platform_windows::{uia, win32, input}` 的公开 API，必然依赖 `platform-windows`；若再让后者反向调用 `cua_driver_qt`，就形成 cargo 依赖环，根本无法编译。上游已提供现成钩子：`DriverHostOptions.register_host_tools: Option<fn(&mut ToolRegistry)>`，在 `build_registry` 末尾执行。CLI、MCP、`tools` 元数据都经过同一处，注册效果等价，且对 `platform-windows` 是**零改动**，升级冲突面比原方案更小。

这样上游 rebase 的冲突面被限制在 `main.rs` 的一行，升级 `cua-driver` 的代价可预测。

---

## 3. 总体架构

### 3.1 三个平面

```text
┌──────────────────────────────────────────────────────────────┐
│  测试语义平面                                                  │
│  casefile (YAML)  →  runner  →  assert  →  evidence / report   │
├──────────────────────────────────────────────────────────────┤
│  归一化平面                                                    │
│  Node / Observation / Identity / ResolvedTarget               │
├──────────────────────────────────────────────────────────────┤
│  感知执行平面                                                  │
│  qt_bridge   |   uia   |   pixel   |   (future) dom           │
│  复用：platform-windows input/capture/uia + ToolRegistry       │
└──────────────────────────────────────────────────────────────┘
```

**分层规则：**

- 上层只能依赖下层的**接口**，不能依赖具体通道。
- 感知层不知道"测试"是什么；语义层不知道"UIA"是什么。
- 新增框架 = 新增一个 Adapter；新增断言 = 注册一个 evaluator；两者都不改执行器。

### 3.2 一次用例执行的完整数据流

```text
cua-driver call qt_run_case '{"case":"cases/smoke.yaml"}'
  │
  ├─ casefile::load         YAML → Vec<Step>（校验 schema、展开变量）
  ├─ runner::run            for each step:
  │    ├─ observe                   → Observation（复用 get_window_state 的 snapshot_id + revision）
  │    ├─ selector::parse           → Selector AST
  │    ├─ resolve::resolve          → ResolvedTarget | Ambiguous | NotFound
  │    ├─ guard::check              → bounds / token 过期 / 风险级 / 确认
  │    ├─ action::do                → ActionResult（effect=confirmed|partial|unverifiable|suspected_noop|refused）
  │    ├─ observe                   → Observation（动作后必须重新观测）
  │    └─ assert::eval              → satisfied | unsatisfied | unknown
  ├─ evidence::write        每步落盘：入参、观察摘要、动作结果、断言结果、耗时
  └─ report::write          JUnit XML + HTML + 差异图
```

**关键约束：**

- 每个 step 开始时重新解析，不复用上一步的 `ResolvedTarget`。
- 动作后必须重新观测；旧 `ResolvedTarget` 立即失效。
- 断言基于**新的观测**求值，不读动作返回值。
- 失败不自动重放整个 step；先重新观测，再决定是否重试。

### 3.3 进程内模块关系

```text
cua-driver (CLI)  ── cua-driver call <tool> ──┐
cua-driver mcp    ── MCP tools/call ──────────┤
                                              ▼
                              ToolRegistry（上游，唯一注册点）
                                 ├─ get_window_state / click / type_text / verify_state ...（原样）
                                 └─ qt_*（本项目 crate 注册）
                                       ├─ resolve
                                       ├─ act            → 背景点击/输入 / UIA Pattern
                                       ├─ assert
                                       └─ run_case ──┬─ casefile
                                                     ├─ runner
                                                     ├─ evidence
                                                     ├─ report
                                                     └─ adapter ── qtbridge / uia / pixel
```

现有工具不删、不改签名；`qt_*` 是同级的普通工具，因此天然同时暴露给 CLI 与 MCP，不需要新增 CLI 子命令。需要人类友好的入口时，V2 再加 `cua-driver qt ...` 薄封装。

---

## 4. 模块设计

### 4.1 归一化节点模型（L1，核心）

所有通道先转成同一个模型，再交给选择器。

```rust
// crates/cua-driver-qt/src/model.rs
use std::collections::{BTreeMap, HashMap};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Source { QtBridge, Uia, Pixel, Dom }

#[derive(Clone, Copy, Debug, Default)]
pub struct Rect { pub x: f64, pub y: f64, pub w: f64, pub h: f64 } // 窗口本地逻辑像素

pub struct Node {
    pub node_id: String,               // 观测内句柄，仅本次 Observation 有效
    pub parent_id: Option<String>,     // None 表示根
    pub role: String,                  // button / edit / listitem / window / canvas ...
    pub name: Option<String>,          // 无障碍名称
    pub automation_id: Option<String>, // UIA AutomationId
    pub class_name: Option<String>,    // QPushButton / QQuickItem / ...
    pub object_name: Option<String>,   // Qt objectName（仅桥）
    pub rect: Rect,
    pub visible: bool,
    pub enabled: bool,
    pub states: BTreeMap<String, String>, // checked / selected / expanded / value
    pub actions: Vec<String>,             // invoke / toggle / set_value / expand ...
    pub source: Source,
    pub attrs: BTreeMap<String, String>,  // 通道私有属性，选择器用 attr(...) 访问
    pub children: Vec<String>,
}

pub struct Observation {
    pub snapshot_id: String,   // 复用上游 "sXXXXXXXX"
    pub element_tokens: HashMap<String, String>, // node_id → element_token
    pub window: WindowRef,
    pub revision: u64,         // 本项目新增：同一窗口的单调序号
    pub captured_at: SystemTime,
    pub space: &'static str,   // 固定 "window_logical"
    pub nodes: Vec<Node>,
    pub by_id: HashMap<String, usize>,
    pub screenshot: Option<ImageRef>,
    pub sources: BTreeMap<Source, usize>, // 各通道贡献节点数，用于降级诊断
    pub degraded: Option<String>,         // 非空表示部分通道失败
    pub warnings: Vec<String>,
}
```

`snapshot_id` / `element_tokens` 直接取上游 `get_window_state` 的返回值，不重新定义语义；`revision` 是新增的排序序号，缺失时退回 `snapshot_id` 的哈希序（见 §4.3）。

#### 坐标空间

**唯一约定：进入 `Node.rect` 之前必须换算到"窗口本地逻辑像素"。** 换算集中在 `crates/cua-driver-qt/src/model/coord.rs`，并配表驱动测试。上游 `UiaNode.rect` 是屏幕坐标，复用其 `frame` 时只做换算，不改上游结构。

| 来源 | 原始空间 | 换算 |
| --- | --- | --- |
| UIA `BoundingRectangle` | 屏幕物理像素 | 减窗口物理原点 → 除 DPI 缩放 |
| WGC 截图 | 截图像素 | 减窗口内偏移 → 除捕获缩放比 |
| Qt 桥 | 屏幕逻辑像素 | 减窗口逻辑原点 |
| 动作坐标 | 窗口本地逻辑像素 | 原样 |

**不变量：** 节点矩形必须落在窗口矩形内（允许 ±2 px 容差）；越界节点丢弃并计入 `Warnings`，同时把 `Degraded` 设为 `coordinate_out_of_bounds`。这条不变量能在早期抓住 DPI 错配，而不是等到点击错位。

### 4.2 选择器语法与匹配（L2）

```ebnf
selector   = call | path | region | uia ;
call       = ident "(" [ arg { "," arg } ] ")" ;
arg        = ident "=" value ;
path       = "path(" segment { ">" segment } ")" ;
region     = "region(" "image=" string [ "," "x=" number "," "y=" number ] ")" ;
uia        = "uia(" "name=" string [ "," "controlType=" string ] ")" ;
```

示例：

```text
widget(objectName="scanStartButton")
widget(className="QPushButton", text="开始扫描")
widget(className="QComboBox", within=widget(objectName="patientPanel"), index=0)
qml(id="scanButton")
path(QMainWindow > QTabWidget > QWidget#scanPage > QPushButton)
region(image="viewport.png", x=0.5, y=0.3)
```

**匹配流水线：**

1. **作用域**：有 `within` 先解析内层，得到候选子树。
2. **硬过滤**：字段完全匹配（区分大小写，`objectName` 与 `AutomationId` 精确匹配）。
3. **软打分**：对未硬性要求但存在的字段加权。
4. **排序与裁决**：见下。

**打分权重：**

| 字段 | 权重 | 说明 |
| --- | --- | --- |
| `objectName` | 0.40 | Qt 最强身份 |
| `qml id` | 0.38 | QML 对象的 `id` 经 `Accessible.name` 或桥暴露 |
| `AutomationId` | 0.30 | UIA 桥实际暴露时才计入 |
| `className` | 0.15 | 类型约束 |
| `accessibleName` / `text` | 0.10 | 易受 i18n 影响 |
| 结构路径完全匹配 | 0.08 | 布局稳定时有效 |

**裁决规则（FR-03 的落地）：**

```text
require  score >= 0.60
require  top1.score - top2.score >= 0.20   // 存在 top2 时
require  len(matches) == 1
否则     → ambiguous_target / target_not_found，附候选列表
```

**绝不"取第一个"，绝不退化成坐标点击。** 误点一个"删除病例"比用例失败贵得多。

**解析结果：**

```json
{
  "matchedBy": ["objectName", "className"],
  "score": 0.86,
  "fingerprint": "sha256:7f3c...",
  "rect": {"x": 412, "y": 288, "w": 96, "h": 32},
  "nodeId": "n37",
  "elementToken": "s0000002a:37",
  "source": "qt_bridge",
  "snapshotId": "s0000002a",
  "revision": 42,
  "alternatives": [
    {"nodeId": "n61", "score": 0.41, "summary": "QPushButton \"开始扫描\" @ (612,288)"}
  ]
}
```

### 4.3 身份与失效（L3）

**身份 = 指纹，句柄 = 编号。** 指纹用于跨观测确认"还是同一个控件"；句柄只用于本次观测内寻址。

**令牌作用域（实现事实）：** 平台的 `SnapshotStore` 属于 `platform-windows` 私有 `ToolState`，外部 crate 拿不到。`qt_resolve` 因此持有自己的 `Snapshots` store（`platform_windows::uia::snapshot::Snapshots`，公开类型），返回的 `snapshotId` / `elementToken` 只对 `qt_*` 有效，不能交给上游 `click` / `set_value`。跨层动作由后续 `qt_act` 用自己的 store 校验，或重新调用 `get_window_state` 获取平台令牌。全局快照计数器共享，所以两套令牌不会重号。

**可观测 ≠ 可操作（实现事实）：** 观测树收两类节点：有可缓存 UIA Pattern 的节点（`nodeId = n{上游element_index}`，带 `elementToken`），以及只有名字/值、没有 Pattern 的节点（`nodeId = u{走查序号}`，`elementToken = null`）。后者正是 Qt 自绘控件在 UIA 下的常见形态（例如 Qt Creator 的"运行"按钮），测试断言需要看见它们；但 `qt_act` 必须只接受前者，不能把 `u*` 当可点击句柄。`elements_complete`、越界丢弃与坐标归一化对两类节点一视同仁。

```rust
// crates/cua-driver-qt/src/model/resolved.rs
pub struct ResolvedTarget {
    pub node_id: String,       // 本次观测句柄
    pub element_token: String, // 上游令牌，动作层直接校验
    pub fingerprint: String,   // 跨运行身份
    pub snapshot_id: String,
    pub revision: u64,
    pub rect: Rect,
    pub source: Source,
    pub selector: String,      // 原样回填，便于报告定位
    pub match_score: f64,
    pub matched_by: Vec<String>,
}
```

**与上游令牌的关系：** 上游 `element_token` 形如 `sXXXXXXXX:N`，动作工具已内置过期拒绝（`element_token is stale; call get_window_state again to refresh`）。本项目不重造令牌，只在它外面补 `fingerprint`（跨运行身份）与 `revision`（同一窗口的单调序号）。`revision` 由本项目在每次成功观测后自增并随结构化输出返回；上游 rebase 后若已有等价字段，则以上游为准。

**指纹构成（按稳定性排序）：**

1. `source + objectName`（P0，最稳）
2. `source + qml id`
3. `source + AutomationId + Role`
4. 结构路径：`role/className` 链 + 同层同类序号
5. 文本 + 角色（弱，仅告警不阻断）

**失效判定表：**

| 条件 | 判定 | 错误码 |
| --- | --- | --- |
| `snapshot_id` 不是当前快照 | 句柄失效 | `stale_snapshot` |
| `revision` 落后于当前 | 观测过期 | `stale_revision` |
| 窗口 bounds 变化 | 坐标不可信 | `bounds_changed` |
| 节点不在当前树中 | 元素消失 | `target_not_found` |
| 指纹与当前节点不符 | 元素被替换 | `identity_mismatch` |

**策略：**

- 每个 step 重新解析，不跨 step 复用 `ResolvedTarget`。
- 同一步内允许"解析一次、连续多个无布局变化的动作"，但每次动作后仍要刷新观测并校验 `revision`。
- `identity_mismatch` 直接失败，不自动重试。元素被替换意味着界面状态已经与用例假设不同。

### 4.4 动作与效果分级（L4）

**动作阶梯（降级链）：**

| 顺序 | 方式 | 适用 | 读回证据 |
| --- | --- | --- | --- |
| 1 | Qt 桥 `invoke` / `setValue` | 桥可用且控件支持 | 桥回读属性 |
| 2 | UIA Pattern（Invoke / Toggle / SelectionItem / Value） | 标准控件 | UIA 回读 |
| 3 | 后台点击 / 后台输入 | 任意可见控件 | 变化前后 diff（弱） |
| 4 | 前台点击 / 前台输入 | 后台无效时显式开启 | 变化前后 diff |
| 5 | 像素坐标 | 自绘区域 | 仅视觉断言 |

**结果分级（复用 cua 的语义，不改字段名）：**

动作底层直接复用上游工具：`click` / `double_click` / `type_text` / `press_key` / `set_value` / `scroll`，以及三态校验 `verify_state`。本项目的 `qt_act` 只负责"选路 + 聚合读回 + 归类 effect"。

| `effect` | 含义 | 判定依据 |
| --- | --- | --- |
| `confirmed` | 已确认生效 | 有读回值或窗口结构性变化证据 |
| `partial` | 部分生效 | 多项预期只完成一部分 |
| `unverifiable` | 已投递但无法确认 | 只确认了输入送达，没有读回 |
| `suspected_noop` | 可读状态未变化 | 有界读回前后一致，**仅供参考** |
| `refused` | 未执行 | 权限、歧义、bounds、过期 |

```json
{
  "effect": "confirmed",
  "route": "qt_bridge",
  "delivery": {"mode": "background"},
  "evidence": [{"kind": "value_readback", "detail": "progress == 100"}],
  "hint": null,
  "next_call": null
}
```

**硬规则：**

- 事件被系统接收 ≠ `confirmed`。
- 截图发生变化 ≠ `confirmed`。
- `unverifiable` 不是失败，也不是成功；由断言决定。
- `refused` 必须带 `code` 与 `hint`；`next_call` 只能是安全的读操作。

### 4.5 断言（L6）

断言对**动作后的新观测**求值，分三态：

| 状态 | 含义 | 用例结果 |
| --- | --- | --- |
| `satisfied` | 有证据证明成立 | 通过 |
| `unsatisfied` | 有证据证明不成立 | 失败 |
| `unknown` | 观测不足 | **失败**（并标记 `unknown`，便于区分环境问题） |

**复用上游实现（有边界）：** `verify_state` 的三态语义与轮询机制（稳定采样默认 2 次、`MAX_PREDICATES=8`、10 s 超时）直接沿用。但它的 `ElementSelector` 只有 `role` 与 `label_contains`，`ElementPredicate` 只有 `exists` / `value_equals` / `enabled` / `selected`，且拒绝 `exists:false`。因此：

- 内置断言先在归一化观测上求值，使用完整的 Qt 选择器；
- 当选择器可等价映射为 `role + label_contains` 时，可委托 `verify_state` 获得上游的轮询与稳定采样；
- `notExists` 必须同时满足 `elements_complete=true`，否则返回 `unknown`，绝不返回 `satisfied`。

**内置断言类型：**

| `kind` | 参数 | 说明 |
| --- | --- | --- |
| `exists` | `target` | 解析唯一即成立 |
| `visible` / `enabled` | `target` | 读节点状态 |
| `text` | `target`, `equals` / `matches` / `contains` | 文本比较 |
| `value` | `target`, 同上 | Value Pattern / 桥属性 |
| `checked` | `target`, `equals` | 勾选态 |
| `count` | `target`, `equals` | 子项数量（列表行数） |
| `property` | `target`, `name`, `equals` | 桥暴露的任意只读属性 |
| `signal` | `name`, `timeout` | 桥订阅信号（v2） |
| `visual` | `region`, `baseline`, `threshold` | 视觉回归 |
| `latency` | `step`, `max` | 步骤耗时上限 |

断言失败必须带：`expected`、`actual`、`evidence`（截图 + 树 dump + 匹配打分）。

### 4.6 用例模型与编排（L6）

```yaml
name: 扫描流程冒烟
app: ScanApp.exe
args: ["--test-mode"]
tags: [smoke, scan]
fixture: case_simple_001
timeout: 5m

steps:
  - action: launch
    timeout: 30s

  - action: click
    target: widget(objectName="openCaseButton")

  - action: waitFor
    target: widget(objectName="scanStartButton")
    state: visible
    timeout: 10s

  - action: click
    target: widget(objectName="scanStartButton")

  - assert:
      kind: value
      target: widget(objectName="progressLabel")
      equals: "100%"

  - assert:
      kind: visual
      region: region(image="viewport.png", x=0.5, y=0.3)
      baseline: viewport_base
      threshold: 0.02

teardown:
  - action: click
    target: widget(objectName="closeCaseButton")
```

**执行规则：**

| 规则 | 内容 |
| --- | --- |
| 变量 | `$fixture`、`$env`、`$case.dir`，只允许白名单来源 |
| 重试 | 默认 0；只允许对 `waitFor`、观测类步骤显式重试；动作步骤重试必须重新解析 + 重新观测 |
| 超时 | 步骤 15 s，用例 5 min，可覆盖 |
| 失败 | 当前用例立即停止，保留现场；连续 3 条用例环境性失败则中止整批 |
| 并行 | 同一 exe 的多个实例可并行；同一窗口不可并行 |

**为什么允许 `waitFor` 重试、不允许盲目重放动作：** 等待是幂等的；点击不是。重放一个"已生效但未确认"的点击，可能变成双击或重复提交。

### 4.7 证据与报告（L6）

在需求文档 FR-07 的基础上补充动作契约字段：

```text
runs/2026-11-05T10-31-02/
  report.xml                 # JUnit，CI 直接消费
  report.html                # 人读，含失败截图与树
  case.json                  # 用例元信息、环境、版本、DPI
  steps/0001.json            # 入参、selector、matchedBy、score
                             # effect/route/delivery/evidence、revision、耗时
  observations/0001.json     # 归一化节点摘要（不含大字段）
  trees/0001.txt             # 原始树 dump
  shots/0001.png
  diff/0001-baseline.png
  diff/0001-actual.png
  diff/0001-diff.png
```

**保留策略：** `--keep-evidence=never|on-failure|always`，默认 `on-failure`。失败证据永不自动清理。

### 4.8 视觉回归（L7）

- **ROI 化**：默认只比 `region(...)` 指定的区域，不做全窗口比较。
- **掩膜**：光标、时间戳、进度动画、闪烁元素支持矩形掩膜。
- **指标**：像素差比例 + SSIM，双阈值；任一超限即失败。
- **确定性前置条件**（缺一不可）：
  1. `--test-mode` 关闭动画；
  2. 固定窗口尺寸；
  3. 固定 DPI 与缩放；
  4. 同一渲染后端（软件渲染优先，避免 GPU 驱动差异）。
- **基线**：仓库或制品库存放，带版本、哈希、审批记录；`--update-baseline` 默认禁止，且必须在报告中留痕。

**边界：** 视觉回归不能证明"功能正确"，只能证明"渲染结果与基线一致"。3D 视口如果拿不到状态 API，视觉断言是唯一手段，但必须同时接受"驱动差异导致的误报"。

### 4.9 Qt 测试桥（L5，可选组件）

**形态：** 应用内插件，由应用自己在 `--test-mode` 或 `QT_TEST_BRIDGE=1` 时加载。不做远程注入。

**传输：** 命名管道 `\\.\pipe\qt-test-bridge-<pid>`，行分隔 JSON-RPC 2.0，握手时校验一次性 token（由启动器生成并写入子进程环境）。

**方法（v1）：**

| 方法 | 能力 | 读/写 |
| --- | --- | --- |
| `hello` | 版本、Qt 版本、进程、窗口列表 | 读 |
| `snapshot` | QObject 树、几何、属性白名单 | 读 |
| `resolve` | 按 `objectName` / 路径解析 | 读 |
| `getProperty` | 单个只读属性 | 读 |
| `invoke` | 触发 QAction / 点击 / 切换 | 写，白名单 |
| `setValue` | 设置输入框值 | 写，白名单 |

**线程模型：** 所有 QObject 访问必须回到 GUI 线程。桥内部统一用 `QMetaObject::invokeMethod(obj, ..., Qt::BlockingQueuedConnection)`，绝不在管道线程直接读控件。

**快照节点：**

```json
{
  "objectName": "scanStartButton",
  "className": "QPushButton",
  "metaClass": "QPushButton",
  "accessibleName": "开始扫描",
  "role": "button",
  "rect": {"x": 412, "y": 288, "w": 96, "h": 32},
  "visible": true,
  "enabled": true,
  "states": {"checked": "false"},
  "actions": ["invoke"],
  "attrs": {"text": "开始扫描", "toolTip": ""}
}
```

**降级矩阵：**

| 场景 | 有桥 | 无桥 |
| --- | --- | --- |
| QWidget 标准控件 | P0 `objectName` | P1/P2 UIA |
| QML 控件 | P0 `objectName` / `id` | P1 `Accessible.name`，可能缺失 |
| 自绘控件 | P0 `objectName` + 自定义属性 | 仅 P5 坐标 + 视觉断言 |
| 3D / OpenGL 视口 | 若应用暴露只读状态，可断言 | 仅视觉断言 |

**安全：**

- 只在显式测试模式加载；生产构建可完全剥离（编译开关）。
- 只监听本机管道，token 校验失败立即断开。
- 写操作按 `objectName` 白名单；默认只读。
- 不提供文件、网络、进程、注册表能力。

### 4.10 权限与安全（L8）

不移植 7 层策略，只保留测试场景真正需要的三层：

| 级别 | 例子 | 默认 |
| --- | --- | --- |
| `observe` | 快照、解析、断言、截图 | 允许 |
| `mutate_safe` | 点击、输入、切换、滚动 | 允许（需 `--test-mode` 目标） |
| `destructive` | 删除、上传、提交、关闭未保存、终止进程 | 动作时确认；用例里必须显式声明 `allowDestructive: true` |

**硬拒绝（不可配置绕过）：**

- 密码管理器、终端、系统安全/隐私设置界面；
- 认证与支付对话框；
- 目标进程不是声明的被测进程（防止误操作到用户正在用的窗口）；
- 窗口标题与声明不符时默认拒绝。

**审计：** 每次 `destructive` 动作写入 `audit.json`，包含时间、目标、选择器、操作者（用例名）。

---

## 5. 关键数据契约

### 5.1 Observation（摘要）

```json
{
  "snapshotId": "s0000002a",
  "revision": 42,
  "window": {"app": "ScanApp.exe", "id": 266852, "title": "扫描工作站"},
  "space": "window_logical",
  "capturedAt": "2026-11-05T10:31:07.412Z",
  "invalidatedSnapshotIds": ["s00000029"],
  "sources": {"qt_bridge": 412, "uia": 0, "pixel": 0},
  "degraded": "",
  "nodes": [
    {
      "nodeId": "n37",
      "elementToken": "s0000002a:37",
      "parentId": "n12",
      "role": "button",
      "name": "开始扫描",
      "objectName": "scanStartButton",
      "className": "QPushButton",
      "rect": {"x": 412, "y": 288, "w": 96, "h": 32},
      "visible": true,
      "enabled": true,
      "actions": ["invoke"],
      "source": "qt_bridge"
    }
  ]
}
```

`snapshotId` 与 `elementToken` 采用上游格式（`sXXXXXXXX` 与 `sXXXXXXXX:N`）；`revision` 是本项目增量字段。

### 5.2 StepResult

```json
{
  "index": 4,
  "action": "click",
  "selector": "widget(objectName=\"scanStartButton\")",
  "matchedBy": ["objectName", "className"],
  "score": 0.86,
  "fingerprint": "sha256:7f3c...",
  "revisionBefore": 41,
  "revisionAfter": 43,
  "result": {
    "effect": "confirmed",
    "route": "qt_bridge",
    "delivery": {"mode": "background"},
    "evidence": [{"kind": "value_readback", "detail": "progress == 100"}]
  },
  "durationMs": 212,
  "ok": true
}
```

### 5.3 CaseResult

```json
{
  "name": "扫描流程冒烟",
  "status": "passed",
  "startedAt": "2026-11-05T10:31:02Z",
  "durationMs": 8420,
  "steps": 7,
  "failedStep": null,
  "evidenceDir": "runs/2026-11-05T10-31-02",
  "environment": {"dpiScale": 1.25, "qtVersion": "6.6.2", "bridge": true}
}
```

---

## 6. 二次开发扩展点

"可通用 + 可二次开发"落在六个注册表上。新增能力不改内核。

| 扩展点 | 接口 | 用途 | 注册方式 |
| --- | --- | --- | --- |
| 感知适配器 | `Adapter` | 接入新框架（WPF、Electron、Flutter） | `Registry::adapters.register(name, factory)` |
| 选择器方言 | `SelectorProvider` | 新增 `uia(...)`、`css(...)` 等语法 | `Registry::selectors.register("uia", parser)` |
| 动作驱动 | `ActionDriver` | 新增 invoke 策略、效果判定 | `Registry::actions.register(route, driver)` |
| 断言 | `Evaluator` | 新增断言类型 | `Registry::assertions.register(kind, evaluator)` |
| 报告 | `Reporter` | JUnit / HTML / Allure / 内部平台 | `Registry::reporters.register(name, reporter)` |
| 用例来源 | `CaseSource` | YAML / JSON / CSV / Excel 导入 | `Registry::cases.register(name, loader)` |

```rust
// crates/cua-driver-qt/src/adapter.rs
#[async_trait::async_trait]
pub trait Adapter: Send + Sync {
    fn name(&self) -> &'static str;
    /// 让上层知道该适配器能提供哪些字段与动作。
    fn capabilities(&self) -> Capabilities;
    /// 返回归一化节点；实现方负责把坐标换算到窗口逻辑像素。
    async fn observe(&self, window: &WindowRef) -> Result<Observation, AdapterError>;
    /// 只处理本适配器能证明效果的动作用途。
    async fn invoke(&self, target: &ResolvedTarget, action: Action)
        -> Result<ActionResult, AdapterError>;
}

// crates/cua-driver-qt/src/assert.rs
pub trait Evaluator: Send + Sync {
    fn kind(&self) -> &'static str;
    fn eval(&self, input: &EvalInput) -> EvalResult;
}
```

**与上游扩展点的关系：** 六个注册表是"测试语义层"的扩展点，不替换上游的 `ToolRegistry`。上游 `ToolRegistry` 负责"有哪些工具"，本项目注册表负责"工具内部怎么选路、怎么判定"。两者正交，新增断言不需要改工具，新增工具也不需要改断言。

**稳定性承诺：**

- `Adapter` 与 `Evaluator` 接口按语义版本管理，破坏性变更升主版本。
- 报告与证据是**纯 JSON**，外部平台可以直接消费，不需要链接 Rust 代码。
- 新增工具先以 runtime schema 上线（`ToolDef` 直接构造），契约稳定后再迁入 `cua-driver-contract`，避免每次改字段都重编 UniFFI 绑定。
- 进程外插件（JSON-RPC over stdio）列为 v2，v1 不做，避免过早抽象。

**通用性的验收方式：** 用一个非 Qt 的 Win32 记事本做一条冒烟用例，只通过 UIA Adapter 跑通。跑通即证明内核没有写死 Qt；跑不通说明分层漏了。

---

## 7. 代码结构

新增一个 workspace crate，注册进现有平台 runtime；上游只改一个注册点。

```text
libs/cua-driver/rust/
  Cargo.toml                     # workspace members 加一行 crates/cua-driver-qt
  crates/cua-driver-qt/          # 新增 crate（MIT）
    Cargo.toml
    src/
      lib.rs                     # pub fn register_qt_tools(&mut ToolRegistry)
      tools/
        resolve.rs               # tool: qt_resolve
        act.rs                   # tool: qt_act
        run_steps.rs             # tool: qt_run_steps
        assert.rs                # tool: qt_assert
        run_case.rs              # tool: qt_run_case
        observe_ext.rs           # 给 get_window_state 增补 revision/归一化视图（只读）
      model.rs coord.rs          # Node / Observation / Rect / 坐标换算
      adapter/
        mod.rs                   # Adapter trait + 注册表
        uia.rs                   # 包装 platform-windows 的 UiaNode
        qtbridge.rs              # 命名管道客户端
        pixel.rs                 # 仅坐标与视觉，不产出语义节点
      selector/                  # lexer.rs parser.rs ast.rs match.rs score.rs
      identity/                  # fingerprint.rs invalidate.rs
      action/                    # ladder.rs effect.rs confirm.rs
      assert_impl/               # eval.rs kinds.rs visual.rs（复用上游 verify_state）
      casefile/                  # schema.rs loader.rs vars.rs
      runner/                    # runner.rs retry.rs timeout.rs
      evidence/                  # writer.rs redact.rs
      report/                    # junit.rs html.rs
      visual/                    # roi.rs diff.rs baseline.rs
    tests/                       # 契约测试：qt_* 与既有工具共存、令牌过期、三态断言
  crates/cua-driver/src/main.rs  # register_host_tools() 末尾加一行：
                                 # cua_driver_qt::register_qt_tools(registry);
                                 # （通过 DriverHostOptions.register_host_tools 钩子生效）
  bridges/qt-test-bridge/        # C++ 插件（CMake + qmake 两种接入示例）
    include/qt_test_bridge.h
    src/bridge.cpp src/snapshot.cpp src/invoke.cpp
    examples/cmake/ examples/qmake/
skills/qt-test-agent/
  SKILL.md
  references/selectors.md
  references/assertions.md
  references/recovery.md
  references/safety.md
```

**依赖方向：** `tools → runner/selector/action/assert_impl → adapter → model`。`model` 不依赖其他模块；`adapter` 不依赖 `runner`；`cua-driver-qt` 依赖 `cua-driver-core`（`Tool`/`ToolRegistry`/`expectation`）与 `platform-windows`（UIA/输入/捕获），**反向不依赖**。

**命名空间：** 所有新工具统一 `qt_` 前缀，避免与上游工具名冲突；上游升级新增工具不会撞名。

---

## 8. 工作量估算

单位：人日。假设 1 名熟悉 Rust 的工程师 + 应用团队配合。若工程师不熟 Rust，首期需加 5–8 人日学习与联调成本。

| 工作项 | 人日 | 依赖 |
| --- | --- | --- |
| 装工具链、构建 `cua-driver`、读通 Tool/Registry 与平台分层 | 6 | — |
| 新增 crate + 注册 + 工具骨架（`qt_*` 四个） | 4 | 上项 |
| 归一化模型 + 坐标换算 + 单测 | 8 | — |
| 选择器解析 / 打分 / 歧义裁决 | 12 | 模型 |
| 身份指纹 + 失效 + 类型化错误 | 6 | 模型 |
| 动作阶梯 + 效果分级（复用 click/type_text/set_value/verify_state） | 6 | 模型、身份 |
| 断言 + 用例模型 + runner | 14 | 选择器、动作 |
| 证据 + JUnit + HTML 报告 | 6 | runner |
| 视觉回归（ROI / SSIM / 基线） | 7 | runner |
| Qt 桥（C++） + 两种构建示例 | 16 | 应用配合 |
| 技能包 + 文档 + 样板用例 | 5 | 全部 |
| 联调 / 稳定性打磨（20 次回归） | 10 | 全部 |
| **合计** | **100** | |

**折算：** 单人约 4.5–5 个月；两人并行（Rust 内核 + Qt 桥）约 9–11 周。该估算已回写到需求文档 v0.3 的里程碑；前提是应用团队已经准备好 `objectName` 与 `--test-mode`，且开发机已装 Rust 工具链（当前未装）。

**应用团队配合（不由本项目承担）：**

| 事项 | 人日 |
| --- | --- |
| 关键控件补 `objectName` / `Accessible.name` | 3–5 |
| `--test-mode` 启动参数（关动画、固定尺寸、关自动更新） | 2 |
| 3D 视口只读状态接口 | 3–8 |
| 自绘控件 `QAccessibleInterface` | 5–10 |

---

## 9. 风险与对策

| 编号 | 风险 | 概率 | 影响 | 对策 | 早期信号 |
| --- | --- | --- | --- | --- | --- |
| R1 | 应用不允许加桥 | 中 | 高（自绘控件不可测） | 先做 M0 决策；无桥时明确降级并限制承诺 | 应用团队无法在 2 周内给出 `--test-mode` |
| R2 | Qt 桥线程/版本问题导致偶发崩溃 | 中 | 高 | 全部访问走 GUI 线程；只在测试模式加载；加超时与断连恢复 | 首次联调出现随机卡死 |
| R3 | DPI / 多屏坐标错位 | 高 | 中 | 统一坐标空间 + 不变量校验 + 125%/150% 用例 | 首次真机点击偏移 |
| R4 | 自绘 3D 视口无结构、无状态 | 高 | 中 | 视觉断言 + 要求状态 API；不承诺"可定位内部元素" | 视口在树里只有 1 个节点 |
| R5 | 动画/异步导致 flake | 高 | 中 | `--test-mode` 关动画；`waitFor` 代替 sleep；20 次回归门槛 | 第 5 次回归开始偶发失败 |
| R6 | CI 无交互式桌面 | 中 | 高 | 要求交互式登录会话；提供 self-hosted runner 说明 | 流水线里 UIA 树为空 |
| R7 | 选择器随 i18n 漂移 | 中 | 低 | P0 `objectName` 为主，文本仅辅助 | 切换语言后用例批量失败 |
| R8 | 重试掩盖真实缺陷 | 中 | 中 | 动作步骤默认不重试；重试必须留痕并在报告标红 | 报告中 retry>0 比例上升 |
| R9 | 可选感知组件的 AGPL 边界 | 低 | 中 | 不安装/不分发 `cua-perception` 与 `cua-som`；依赖清单与制品审计 | 新依赖或模型包出现 AGPL/FSL |
| R10 | Rust 工具链缺失 / 版本漂移 | 高 | 低 | 首日安装 rustup 并锁定 `rust-toolchain.toml`；CI 使用同一版本 | `cargo build` 在本机通过、在 CI 失败 |
| R11 | 上游升级导致注册点或工具行为变化 | 中 | 中 | 只改一个注册点；`qt_*` 独立 crate；升级前跑契约测试（令牌过期、三态断言、工具共存） | rebase 出现注册函数签名冲突 |
| R12 | 新工具 schema 与 contract 不一致 | 中 | 低 | 首期只走 runtime schema；迁 contract 时用 `SchemaMode::CanonicalRuntime` 校验，不手写第二份 | `list_tools` 输出与文档不一致 |

---

## 10. 路线与里程碑

| 阶段 | 内容 | 退出条件 |
| --- | --- | --- |
| M0（1 周） | 决策：能否加桥、Qt 版本、QML 占比、CI 桌面；装 Rust 并构建 `cua-driver` | Q1–Q5 有明确答案；`cargo test -p platform-windows` 通过；`cua-driver list-tools` 可运行 |
| M1（2 周） | 新增 crate + 注册 + 归一化模型 + UIA Adapter + 选择器 + `qt_resolve` | 30 控件样板应用 P0/P1 解析 100% |
| M2（2 周） | 身份/失效 + 动作阶梯 + 效果分级（复用上游动作工具） | 后台点击被遮挡窗口成功，`effect` 分类正确 |
| M3（2 周） | 用例 + 断言（复用 `verify_state`）+ runner + 报告 | 一条 YAML 用例一条 `cua-driver call` 跑完并出 JUnit |
| M4（3 周） | Qt 桥 + 自绘方案 | QWidget/QML/自绘三类各一条用例通过 |
| M5（2 周） | 视觉回归 + CI 接入 | 每日回归稳定，20 次 ≥ 19 次通过 |
| M6（1 周） | 技能包 + 文档 + 打包 | Agent 无提示完成 S1 冒烟 |

**M1 的第一个可验证动作**：任意未最小化的 UIA 窗口（记事本、计算器或 Qt 应用均可）上跑 `cua-driver call qt_resolve '{"pid":<pid>,"window_id":<hwnd>,"selector":"uia(name=\"...\")"}'`，返回带 `elementToken` 与 `revision` 的 `ResolvedTarget`。这同时验证了"注册成功、令牌透传、UIA 适配"三件事；CLI 需先 `cua-driver serve`，或改用 `cua-driver mcp --direct`。

**M1 骨架已验证（2026-11-05）：** 在 Calculator（pid 22020, window 266852）上唯一命中返回带令牌的 `ResolvedTarget`；同一控件连续两次解析，令牌从 `s0000000a:32` 变为 `s0000000b:32`、`revision` 递增，而指纹不变——句柄与身份确实分离。在 Qt Creator（pid 3344, window 68372，scale 1.5）上：`uia(name="Build Project for All Configurations", controlType="Button")` 两次解析令牌变化、指纹 `sha256:5f07…` 不变；`uia(name="运行", controlType="Button")` 命中 Qt 自绘节点，`elementToken: null`、`nodeId: "u15"`，指纹跨观测稳定；窗口关闭按钮落在逻辑坐标右上角（x=1492，窗口逻辑宽 1540），Qt 左下工具栏按钮位置同样吻合，验证了坐标归一化。拒绝语义：`uia(controlType="CheckBox")` → `ambiguous_target`（13 个候选）；不存在的名字 → `target_not_found`（`elementsComplete=true`）；`max_elements=5` 截断走查 → `status:"unknown"`（不把未证明的缺失当作不存在）；最小化窗口 → `window_minimized`；`uia(name=OK)` → `invalid_selector`；`widget(...)` → `unsupported_selector`。测试同时发现并修复了一个真实缺口：Qt Creator 的"运行""开始调试"没有可缓存 UIA Pattern，原先被整条丢弃，现已按"可观测 ≠ 可操作"纳入观测树（见 §2.1 第 4 条与 §4.3）。

**每个里程碑都必须是可演示的端到端切片**，不允许"先写三周库再联调"。

---

## 11. 需求追溯与变更记录

### 11.1 需求到设计的映射

| 需求 | 设计位置 |
| --- | --- |
| FR-01 应用接入与生命周期 | §3.2、§4.6（runner 启动步骤复用现有 runtime） |
| FR-02 观测 | §4.1 归一化模型、§4.3 令牌与失效（复用上游 `snapshot_id`/`element_token`） |
| FR-03 元素解析 | §4.2 选择器与裁决 |
| FR-04 动作 | §4.4 动作阶梯与效果分级（复用上游动作工具） |
| FR-05 断言 | §4.5 三态断言（复用 `verify_state`） |
| FR-06 用例模型 | §4.6 编排与重试规则 |
| FR-07 证据与报告 | §4.7 证据目录与字段 |
| FR-08 稳定性与恢复 | §4.3 失效判定、§4.6 重试规则、§9 R5 |
| FR-09 视觉回归 | §4.8 |
| FR-10 Qt 测试桥 | §4.9 |
| FR-11 CLI / MCP | §3.3 工具注册关系、§7 代码结构（`cua-driver call` + MCP 共用注册表） |
| FR-12 技能包 | §7 目录、§10 M6 |
| FR-13 通用适配器 | §2 ADR-03/04/13/14、§6 扩展点 |

### 11.2 变更记录

| 版本 | 变更 |
| --- | --- |
| v0.1 | 初稿：难度判断、ADR、三平面架构、模块设计、工作量 |
| v0.2 | 文档移入 `docs/qt-test-agent/`；D-1..D-4 与 FR-13 已回写需求 v0.2；里程碑与工期对齐；本章改为需求追溯表 |
| v0.3 | 基线切换为 `cua-driver`（Rust，MIT）：ADR-01/02/06/13 重写；新增 §2.1 复用清单与上游修改策略；§3/§4/§6/§7 改为 Rust 与 `ToolRegistry`；工期 96→100 人日；新增 R10–R12；里程碑接入真实构建命令 |

---

## 12. 一页结论

- **难度：** 6/10。runtime 已存在且比预想完整，新增的是身份、判定、编排；真正的风险是 Qt 桥与非确定性，不是代码量。
- **基线：** `cua-driver`（Rust，MIT）。复用它的令牌、`verify_state`、输入与捕获；不引入 AGPL 感知组件与策略引擎。
- **接入方式：** 新增 `crates/cua-driver-qt` + 一个注册点，`qt_*` 与既有工具同级，CLI 与 MCP 自动可用。
- **架构：** 三平面（感知执行 / 归一化 / 测试语义）+ 六个注册表（Adapter、Selector、ActionDriver、Evaluator、Reporter、CaseSource）。
- **通用性：** 内核与框架无关；Qt 是一个 profile。用一个 Win32 记事本用例反向验证分层。
- **工期：** 100 人日；两人并行 9–11 周；应用团队配合 13–25 人日。
- **第一件事：** 装 Rust 工具链并构建 `cua-driver`，同时确认应用能否加测试桥（Q1）。后者决定项目是 6/10 还是 8/10。
