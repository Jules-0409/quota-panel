# AGENTS.md — quota-panel

桌面常驻的 AI 额度监控小组件（Tauri v2 + Rust）。这是一个**公开的开源项目**，
会被别人 clone 去用，所以按正式项目的标准走。

完整的产品说明在 [README.md](README.md)，这里只写「在这个仓库里干活要知道什么」。

---

## 一、先看这个

- **这是公开仓库**：任何提交都等于对外发布。凭据、密钥、内网地址、
  公司业务数据、以及任何只对某台机器有意义的路径，一律不进仓库。
- **不要把提交历史当垃圾桶**：密钥一旦提交，改掉再推也没用，它还在历史里。
  拿不准就先别提交，先问。
- **push 需要当次明确授权**，授权一次不等于以后每次都能推。
- 本文档面向**任何来改这个项目的人**，包括没见过原作者的外部贡献者：
  不要在这里引用只有项目内部才存在的文档或机器。

## 二、核心约束（改代码前必须知道）

### 1. 只读、不落盘、不伪装

这是产品的立身之本，改任何取数逻辑前先读 README 的「风险与免责声明」。

- **凭据一律只读**。读 Cursor / Devin 的 `state.vscdb` 用 `open_vscdb_readonly()`，
  它内部保证「绝不新建文件」+ 降级时立刻 `PRAGMA query_only`。**不要绕过它直接
  `Connection::open`**。
- **不写、不传、不缓存 token**。凭据只在内存里用完即弃，没有磁盘缓存。
- **User-Agent 如实报自己**（`http.rs` 的 `USER_AGENT`，有测试锁着）。
  2026-09-17 实测过：伪造 UA 没有任何功能收益，纯属白送服务条款违规。
  **不要为了「看起来像官方客户端」去改它。**

### 2. 不做本地缓存兜底（Devin）

Devin 一栏每轮都直接打服务端接口。**接口失败就如实报错，不许拿旧数字冒充。**
历史上 `DevinQuota` 曾有 `stale` / `stale_reason` 字段做本地缓存回退，已经删干净了
（`models.rs` 有测试防复活）。改取数逻辑时不要把这个设计加回来。

### 3. 错误是语言无关的 key

后端 `Err(String)` 一律是 `devin.auth_expired`、`cred.decrypt_key_mismatch`
这种 key，**翻译只在前端做**，两份字典：

- `ui/index.html` 的 `ERROR_TEXT`（界面）
- `bin/quota.rs` 的 `error_text()`（命令行）

**加新错误 = 同时改这三处**（返回 key 的地方 + 两份字典），否则界面上会露出
光秃秃的英文 key。新增的界面固定文案同理，要走 `quota.rs` 的 `Ctx::t()`
和 `index.html` 的 `TEXT` 字典——**不许在 `print_*` 里直接写中文字面量**，
`english_mode_has_no_chinese_leftovers` 这个测试会拦住你。

### 4. 前后端契约走 camelCase

`models.rs` 里所有结构体都是 `#[serde(rename_all = "camelCase")]`，
前端按 `r.autoPercentUsed` 这样读。**改字段名会静默导致界面显示「无数据」**
（不会报错），所以 `models.rs` 里有一份契约测试钉着，改字段名会红。

### 5. 两个 bin 的关系

- `quota-panel-tauri`（主程序）：GUI 子系统，**Windows 上没有控制台**。
- `quota`（CLI）：必须单独声明 `[[bin]]`，和 GUI 共用 `quota_panel_lib::query_for_cli()`。

**不要在 CLI 里重新实现取数逻辑**，也不要把 `windows_subsystem = "windows"`
加到 `quota` 上——加了就再也打印不出东西了（`main.rs` 的 `cfg_attr` 只作用于主程序）。

## 三、验证方式（每次改完都要跑）

```powershell
# 在 quota-panel-tauri/src-tauri 下。注意这台机器 git / cargo 不在默认 PATH 里
cd D:\code2\quota-panel\quota-panel-tauri\src-tauri
& "$env:USERPROFILE\.cargo\bin\cargo.exe" test --bins --lib
& "$env:USERPROFILE\.cargo\bin\cargo.exe" clippy --all-targets -- -D warnings
```

**测试是真凭据的替代品**：这个项目的取数逻辑依赖本机登录态，
CI 上跑不了真实接口，所以**纯函数必须有单元测试**——
日期解析、protobuf 编解码、GCM 解密、重试判定、契约序列化、CLI 格式化。
这几处的测试已经抓到过真 bug（见 `PROGRESS.md`），不要为了省事删掉。

**界面改动必须过眼睛**：透明窗口 + `backdrop-filter` 的坑很多，
日志全绿也可能样式全丢。改 `ui/index.html` 后要实际 `cargo run` 看一眼，
或者至少截图。已知的历史坑：定时重建 DOM 会让透明窗口闪烁，
所以倒计时走 `bindReset` 原地改文本，**不要改成整页重渲染**。

## 四、工程习惯

- **代码注释写「为什么」**，不写「做了什么」。现有代码的注释密度是标准，
  尤其是那些看起来奇怪的写法（手搓 GCM、只读降级、`resize_window` 的贴边锚定）
  都配了原因，**改它们之前先读注释**。
- **不静默吞错**。`let _ =` 只用在「失败了也确实无需处理」的地方
  （如 emit 事件、设置窗口属性）。
- 单文件 soft cap ~400 行。`credentials.rs` 和 `cursor.rs` 已经超过，
  再加功能时优先考虑拆文件而不是继续堆。
- `tmp/` 是草稿区，已 gitignore。一次性脚本（如生成测试向量的）放那里，用完可删。
- **不提交** `target/`、`gen/`（已在 `.gitignore`）。

## 五、改契约 / 接口时

这些接口**全是逆向出来的非公开接口**，厂商随时会改：

| 数据源 | 端点 | 备注 |
| --- | --- | --- |
| Factory | `api.factory.ai/api/billing/limits` | JSON + Bearer |
| Devin | `server.codeium.com/.../GetUserStatus` | **protobuf over Connect-RPC**，字段号是猜的 |
| Cursor | `cursor.com/api/usage-summary` | JSON + Bearer + 会话 Cookie |
| Grok Bot | `api2.cursor.sh/aiserver.v1.DashboardService/GetSandUsageStatus` | Connect-RPC，`Connect-Protocol-Version: 1` 是协议声明不是伪装 |

**Devin 那条最容易踩坑**：手写的 protobuf 解析器（`devin.rs`）是按字段号取值的，
厂商加字段不会报错、只会读到错的值。改之前先加/跑 `devin.rs` 的测试，
确认 wire type 和嵌套层级还对得上。解析器对「截断的 length-delimited」
「非法 varint」都已经有防护，**改动时不要放松这些边界**。

不要动 `Connect-Protocol-Version` 头——那是协议版本声明，去掉会 4xx。

---

**改了本文件，要在提交说明里讲清楚改了什么、为什么。**
