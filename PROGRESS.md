# PROGRESS.md — quota-panel

**这个文件是「做到哪了」的唯一权威。** 只看它 + `git log` 就应该能接手。
诚实优先：没验证的标 ⚠️，进度文件骗人比没有更糟。

最后更新：2026-09-17

---

## 当前状态

**可用。** 三个数据源（Factory / Devin / Cursor 含 Grok Bot）在本机实测
都能取到数，`quota` CLI 和 GUI 共用同一套 fetcher。

- 工作树干净；`main` 与 `origin/main` 同步于 `affb83c`，之后有两笔本地提交
  （`5b49365` 拆模块 + 本次记录），**未推送**。
- 2026-09-17 推送的那批提交修掉了下面记录的 3 个 bug、补上了测试和 CI。
  **`c424ac5` 及更早的版本没有这些修复。**
- 推送经过当次明确授权；以后每次推送仍要重新获得授权。

上一次实跑验证（2026-09-17，Windows + 真实登录态）：

- `cargo test --bins --lib` → **52 passed, 0 failed**
- `cargo clippy --all-targets -- -D warnings` → **通过，无 warning**
- `.\target\release\quota.exe`（真接口）→ 三个数据源都有数；
  `QUOTA_LANG=en` 与 `zh` 两种输出都验证过，无中英混排
- Factory `cycle` 类解析交叉验证：用真实 `cycleEnd` 对 Node 的 `Date.parse`，
  逐秒一致
- `node scripts/check-ui.cjs` → 通过（并做过反向测试：往副本里注入
  文案缺失、字段拼错、死代码，脚本都能报出来）
- 界面改动过眼睛：`scripts/make-ui-preview.cjs` 生成离线预览（假数据、不碰真凭据），
  截图存在 `docs/evidence/panel-overview.png`，三张卡、警告/危险配色、
  「无数据」和「额度已耗尽」两个分支都正常
- GitHub Actions 真跑过：四个 job 全绿（`ed407e0`，run 35182960691，
  含新加的 `cargo fmt --check` 步骤）

拆分之后又在 macOS 本机跑过（2026-09-17 晚；只跑测试，没有碰真接口）：

- `cargo test --bins --lib` → **52 passed, 0 failed**（与拆分前一致）
- `cargo clippy --all-targets -- -D warnings` → 通过，无 warning
- `cargo fmt --check` → 通过

## 提交记录

本轮（2026-09-17）新增，全部已推到 `origin/main`：

```
ede50ba Guard the Command import for non-unix platforms, and bump checkout to v5
9411071 Add CI, and reshape the CLI string table to match error_text
745e840 PROGRESS.md: mark the fixes as pushed to origin/main
1fe8c5d Record the commit list and this round's verification in PROGRESS.md
20491f4 Add project instructions, progress log, and UI/test tooling
33e2f38 Document the test suite in both READMEs
5bdbd7b Fix the ISO date and GCM IV parsing bugs, and add unit tests
```

之后又追加了三个提交（**已推送**，2026-09-17，`main` = `ed407e0`）：

```
ed407e0 PROGRESS.md: record the rustfmt pass and the pending docs commits
2a5d7a8 Apply rustfmt and enforce it in CI
b2a468f Make the docs machine-agnostic
90f8f3e Add a README screenshot, and remove internal-only references from the docs
```

这四个提交让仓库可以直接对外分享：删掉了文档里只对作者本机有意义的绝对路径
和内网说明，补了一张假数据截图，把 rustfmt 补齐并把 `cargo fmt --check`
加进 CI。仓库简介和 topics 也已设置。

再之后（2026-09-17 晚，从 macOS 机器上提交）：

```
affb83c Drop a redundant `return` in `load_factory_key`（已推送）
5b49365 Split `credentials.rs` and `cursor.rs` into module directories（本地，未推送）
```

`5b49365` 是纯搬迁：`credentials.rs` 拆成 `credentials/`（6 个文件）、
`cursor.rs` 拆成 `cursor/`（5 个文件），公开路径（`crate::credentials::*`、
`crate::cursor::*`）不变，52 个测试一个不少。这就是 AGENTS.md 里
「文件超 400 行就优先拆」那条。

本轮之前的：

```
c424ac5 Add a `quota` CLI sharing the GUI's fetchers
1fe59f9 Key the Cursor session cookie off the scoped auth id
2201bd0 Add an English UI that follows the system locale
12eee29 Add English README with a dedicated Devin data-path section
0ec90dd Stop periodic DOM rebuilds that flicker the transparent window
26afc18 Initial open-source release of Quota Panel
```

## 最近做完的（2026-09-17 这一轮）

起因：给项目补工程规范，顺手加测试，**结果测试抓到 3 个真 bug**。

### 抓到的 bug（都有回归测试钉住）

1. **日期解析算错半年**（`cursor/time.rs:parse_iso_to_unix`，拆分前在 `cursor.rs`）
   `(153 * (m + 9) % 12 + 2) / 5` 里，Rust 的 `*` 和 `%` 同优先级且左结合，
   实际被解析成 `(153 * (m + 9)) % 12`，月份贡献全错（9 月算成 6）。
   影响：Cursor 账期重置时间、Grok 重置时间全部偏移约半年。
   **实测验证**：修前 `2026-10-08T02:36:39Z` 解出的 unix 与 Node 差半年，
   修后与 Node `Date.parse` 逐秒一致。
   修法：显式加括号 `153 * ((m + 9) % 12)`。

2. **GCM 解不开 ≥32 字节的 IV**（`credentials/gcm.rs:decrypt_factory_payload`，拆分前在 `credentials.rs`）
   - 长度块写成 `b2_bytes[15] = (iv.len() * 8) as u8`，256 被截成 0（应为
     `to_be_bytes()` 写进 `[8..16]`）；
   - 只喂了 IV 的前 16 字节，超过部分被丢掉，违反 NIST SP 800-38D 的
     `GHASH(IV || 0^s || [len(IV)]_64)`。

   16 字节 IV（Droid CLI 当前实际用的）恰好两处都不受影响，所以线上一直没暴露。
   修法：`ghash.update_padded(iv)` + 规范的长度块。
   **测试向量由 Node 的 `crypto.createCipheriv` 生成**
   （Droid CLI 就是 Node 写的，必须拿真实写出方对照）。

3. **CLI 的英文模式其实是中英混排**（`bin/quota.rs`）
   `QUOTA_LANG=en` 时只有窗口名和错误文案是英文，标题/单位/页脚写死中文。
   修法：加 `Ctx::t()` 文案表，全部固定文案改走它。
   回归测试 `english_mode_has_no_chinese_leftovers` 会扫描所有 key 确认无中文。

### 同时做的

- **删死代码**：`DevinQuota.stale` / `stale_reason`（README 明确说没有本地缓存兜底，
  字段恒为 false）、`AppConfig.always_on_top` / `pinned`（无人读）、
  `index.html` 里对应的徽章分支和 CSS、`credentials.rs` 的 `service` 未使用警告。
- **加测试**：52 个，覆盖 GCM 解密（含认证失败/篡改/长度非法等失败路径）、
  ISO 日期解析（含闰年与畸形输入）、protobuf 编解码（含截断/非法 UTF-8/空响应）、
  重试判定与退避表、前后端 camelCase 契约、CLI 格式化与双语输出。
- **补文档**：本文件 + `AGENTS.md` + `.gitignore` 加 `tmp/`。

## 未验证 / 已知缺口 ⚠️

- ⚠️ **公开提交里的作者邮箱仍是真实 Gmail**（`liujufu019@gmail.com`，全部提交都是）。
  GitHub 的 commits API 和 `.patch` 文件都会直接返回它（个人主页上看不到，但提交里是明文）。
  想换成 GitHub 私密邮箱 `284996397+Jules-0409@users.noreply.github.com` 需要改写全部历史
  并强推，这属于改 Git 身份，Droid 的规则不允许它自己动手，要 Jules 亲自跑。
  现成脚本：`tmp/rewrite-emails.ps1`（含备份与指纹核对，跑完可由 Droid 核对并强推）。
- ⚠️ **macOS 侧没跑过真接口**。写这份记录时用的机器是 Windows。
  `credentials/keyring.rs` 的 Keychain 分支、`cursor/auth.rs` 的 `security` 回退、
  `lib.rs` 的 macOS 私有 API 窗口行为 **都还没在有真实登录态的 macOS 上验证**
  （macOS 上跑过的是单元测试，见上面拆模块那一段）。跨平台改动要特别小心。
- ⚠️ **`quota-panel-tauri` 主程序（GUI）本轮没重新实跑**，只跑了 CLI 和测试。
  界面改的是「删掉 stale 徽章」，风险低，但透明窗口的问题靠日志发现不了，
  **下次动 UI 前应先 `cargo run` 看一眼，或至少渲染一次 `scripts/make-ui-preview.cjs`**。
- ⚠️ **Devin 的 protobuf 字段号是猜的**（见 `AGENTS.md` 的「改契约 / 接口时」一节）。
  现在能对上，厂商改协议就会静默读错值。
- ⚠️ **Grok Bot 的分母未公开**，百分比只能看趋势（README 已如实写明）。
- ⚠️ **配置没持久化**：刷新间隔和阈值还写死在代码里。
- ✅ **CI 已经在 GitHub 上跑绿了**（2026-09-17，commit `ede50ba`，run 35176866022）：
  `lint` + `test` 三个平台（ubuntu / windows / macos）**四个 job 全绿**。
  首次运行（`9411071`）lint 曾红过一次：Linux 上 `use std::process::Command`
  成了 unused import（它只被 macOS / Windows 两条凭据分支用到，两边都被 cfg 掉了），
  已在 `ede50ba` 里加 `#[cfg(any(target_os = "macos", target_os = "windows"))]` 修好，
  并在本机用「把两个平台的 cfg 改成恒假」的办法复现过。
- ✅ **`cargo fmt --check` 已启用**：整棵树在 `2a5d7a8` 里跑过一次 `cargo fmt`，
  同时该提交把 `cargo fmt --check` 加进了 CI 的 lint job（第一步），
  并给 toolchain 装上 `rustfmt` 组件。本机 `cargo fmt --check` 现在通过。
  格式化是纯格式改动，已逐文件核对过：去掉空白和 rustfmt 尾逗号后，
  8 个文件里 6 个完全一致，`credentials.rs` 只有 4 处 import 顺序调整
  （rustfmt 重排 `use`），没有逻辑改动。
- **没有 `tests/` 集成测试目录**：所有测试都是 `#[cfg(test)] mod tests` 内联单元测试。
  涉及的纯函数够用，但跨模块的取数流程没有端到端测试（依赖真登录态，CI 上也难做）。

## 下一步（还没做，供接手者选）

1. **配置持久化**：刷新间隔 / 阈值写进配置文件 + 一个设置界面，
   解决 README「已知限制」第一条。
2. **macOS 真机验证**：在真实 macOS 机器上构建并实跑，确认
   Keychain 分支、`.app` 打包、无边框窗口行为。
   （CI 已在 macOS runner 上编译并跑通测试，但那不等于真机上跑得起来。）

## 常用命令

```bash
cd quota-panel-tauri/src-tauri
cargo test --bins --lib
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cargo build --release --bin quota
./target/release/quota              # 三个数据源（Windows 上是 quota.exe）
QUOTA_LANG=en ./target/release/quota --json
```

## 怎么验证「没坏」

本项目取数依赖本机登录态，CI 上跑不了真接口，所以判断标准是：

1. `cargo test` 全绿（纯函数逻辑没坏）
2. `quota.exe` 实跑一次，三个数据源都有数或至少报出可读的错误
   （凭据/网络问题的错误文案是语言无关 key，便于定位）
3. 改了 UI → 实际 `cargo run` 看一眼（透明窗口的坑靠日志发现不了）
