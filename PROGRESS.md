# PROGRESS.md — quota-panel

**这个文件是「做到哪了」的唯一权威。** 只看它 + `git log` 就应该能接手。
诚实优先：没验证的标 ⚠️，进度文件骗人比没有更糟。

最后更新：2026-09-17

---

## 当前状态

**可用。** 三个数据源（Factory / Devin / Cursor 含 Grok Bot）在本机实测
都能取到数，`quota` CLI 和 GUI 共用同一套 fetcher。

- 工作树干净。`origin/main` 在 `de12d7b`（那批已推送、CI 绿）；本地 `main` 比它多出
  2026-09-18 的提交（配置持久化 `6a55bd5` + 这次的 PROGRESS 记录），**等推送授权**。
- 2026-09-17 推送的那批提交修掉了下面记录的 3 个 bug、补上了测试和 CI。
  **`c424ac5` 及更早的版本没有这些修复。**
- 推送经过当次明确授权；以后每次推送仍要重新获得授权。

最近一次实跑验证（2026-09-18，macOS + 真实登录态）：

- `cargo test --bins --lib` → **59 passed, 0 failed**（43 lib + 16 bin；配置那几个单测是新增的）
- `cargo clippy --all-targets -- -D warnings` → 通过，无 warning；`cargo fmt --check` → 通过
- `node scripts/check-ui.cjs` → 通过（TEXT 49 key × 2 语言，ERROR_TEXT 23 key × 2 语言）
- `./target/release/quota`（真接口）→ 三个数据源都有数，中英输出都干净，退出码 0
- GUI 实跑 + 截图核对：胶囊 / 展开面板 / 右边缘贴边锚定 / 托盘图标与菜单 / ⚙ 设置页
- 配置持久化实测：手写 `config.json` 启动后按新阈值上色；设置页的 +/- 真的写盘
  （「危险 −」被后端从 15 夹到 16）
- `cargo tauri build` → `.app` + `.dmg` 都出得来；打出来的 `.app` 实跑正常
  （`lsappinfo` 显示 `UIElement`，不进 Dock）

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
- `node scripts/check-ui.cjs` → 通过

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

再之后（2026-09-17 晚，从 macOS 机器上提交，**全部已推送**）：

```
affb83c Drop a redundant `return` in `load_factory_key`
5b49365 Split `credentials.rs` and `cursor.rs` into module directories
5fa4fc3 PROGRESS.md: record the module split and the latest push
7c23bf9 Fix the UI check and stale references after the module split
```

`5b49365` 是纯搬迁：`credentials.rs` 拆成 `credentials/`（6 个文件）、
`cursor.rs` 拆成 `cursor/`（5 个文件），公开路径（`crate::credentials::*`、
`crate::cursor::*`）不变，52 个测试一个不少。这就是 AGENTS.md 里
「文件超 400 行就优先拆」那条。

`7c23bf9` 修的是这次拆分留下的一个坑：`scripts/check-ui.cjs` 里硬编码了
`src/cursor.rs`，拆完路径就没了，CI 的 `Check UI` 步骤红过一次；同一提交把
两份 README、`scripts/README.md`、两个生成脚本和 `AGENTS.md` 里的旧文件名
也一起清了。CI 已在 `7c23bf9` 跑绿（run 35292810278，`lint` + ubuntu /
windows / macos 三个 `test` job）。

再之后（2026-09-18，macOS 机器，**本地提交，等推送授权**）：

```
6a55bd5 Persist the refresh interval and thresholds, and fix `cargo tauri build`
```

`6a55bd5` 就是下面那一轮的成果：配置持久化 + 让 `cargo tauri build` 真正能跑。

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

## 最近做完的（2026-09-18 这一轮）

- ✅ **macOS 真机验证（CLI）**：`./target/release/quota` 在这台机器上实跑，Factory（Keychain +
  手搓 GCM）、Devin（手写 protobuf）、Cursor + Grok Bot 三路都拿到了真数据，中英输出都干净、
  退出码 0。上面那条「macOS 侧没跑过真接口」的 ⚠️ 由此收尾。
- ✅ **macOS 真机验证（GUI）**：胶囊 → 展开面板（三张卡片，数字与 CLI 一致）→
  右边缘贴边锚定（`(544,236,290,33)` → `(465,236,369,559)`，右边缘固定 834）→
  托盘图标在菜单栏可见、点开是「立即刷新额度 / 退出 Quota Panel」→ ⚙ 设置页，逐项截图核对过。
- ✅ **进度条颜色复核**：Devin 那两根 61% / 67% 的条取色是 `#34c759`（= `--apple-green`），
  阈值 70/90 下判绿正确；之前怀疑「该绿却发黄」是缩略图上的错觉。
- ✅ **配置持久化**（`6a55bd5`，README 已知限制第一条已删）：
  - `config.rs`：`load` / `save` / `sanitize` 三个纯函数 + 7 个单测（round-trip、缺文件、
    坏 JSON、越界夹取、危险线必须严格高于警告线、NaN 兜底、手改文件读时收敛）。
    落盘先写 `config.json.tmp` 再 rename，断电或磁盘满不会留下半截 JSON。
  - 文件在 `app_config_dir()` 下（macOS 是 `~/Library/Application Support/com.quotapanel.app/`），
    **只有三个数字，没有凭据**（凭据仍然只在内存里用完即弃）。
  - `set_config` 先落盘再更新内存，写不进去就整条失败并回 `config.save_failed`
    （两份字典都加了），界面不会出现「显示已保存、重启又变回去」。
  - `AppState` 改到 `setup` 里建（要 `AppHandle` 才知道配置目录），轮询循环改成
    `select!` 等 `config_changed`，改完间隔立刻重新计时。
  - UI 加了 ⚙ 设置页（三个 +/- 步进，点一下立刻写盘），中英两套文案，`check-ui.cjs` 跟着过。
  - **实测**：手写 `{15, 10, 20}` → 启动后卡片按新阈值上色、页脚显示「15m 自动更新」；
    设置页点「警告 +」写盘 `warnPercent: 15.0`，点「危险 −」被后端夹成 `16.0`。
- ✅ **`cargo tauri build` 跑通**（同一提交）：本 crate 有两个 bin 且没有 `default-run`，
  tauri-cli 直接报 `failed to find main binary`；加上 `default-run = "quota-panel-tauri"`
  后 `.app` 和 `.dmg` 都能出（`target/release/bundle/`）。另外 `cargo tauri build`
  生成的 Info.plist 没有 `LSUIElement`，Dock 里会多一个图标，于是在代码里设了
  `ActivationPolicy::Accessory`，`lsappinfo` 实测从 `Foreground` 变 `UIElement`，
  手搓的和 tauri-cli 打的两种 `.app` 观感一致。

## 未验证 / 已知缺口 ⚠️

- ⚠️ **公开提交里的作者邮箱仍是真实 Gmail**（`liujufu019@gmail.com`，全部提交都是）。
  GitHub 的 commits API 和 `.patch` 文件都会直接返回它（个人主页上看不到，但提交里是明文）。
  想换成 GitHub 私密邮箱 `284996397+Jules-0409@users.noreply.github.com` 需要改写全部历史
  并强推，这属于改 Git 身份，Droid 的规则不允许它自己动手，要 Jules 亲自跑。
  现成脚本：`tmp/rewrite-emails.ps1`（含备份与指纹核对，跑完可由 Droid 核对并强推）。
- ✅ ~~macOS 侧没跑过真接口~~ 2026-09-18 已实跑并核对（见上面那一轮）：Keychain 分支、
  `security` 回退、`lib.rs` 的 macOS 私有 API 窗口行为都在有真实登录态的机器上验过。
- ✅ ~~`quota-panel-tauri` 主程序（GUI）本轮没重新实跑~~ 2026-09-18 已实跑（胶囊 / 展开面板 /
  贴边锚定 / 托盘菜单 / 设置页）。改 UI 仍然要实际渲染一次再提交：透明窗口的坑靠日志发现不了。
- ⚠️ **Devin 的 protobuf 字段号是猜的**（见 `AGENTS.md` 的「改契约 / 接口时」一节）。
  现在能对上，厂商改协议就会静默读错值。
- ⚠️ **Grok Bot 的分母未公开**，百分比只能看趋势（README 已如实写明）。
- ✅ ~~配置没持久化~~ 2026-09-18 已解决（`6a55bd5`，见上面那一轮）。
- ⚠️ **`.dmg` 只验证到「生成成功」**：没有真的挂载安装一遍，也没在干净用户下装过。
- ⚠️ **identifier 还是 `com.quotapanel.app`**（以 `.app` 结尾），tauri-cli 会警告它和
  bundle 后缀冲突。改它要同时迁配置目录和登录项，越晚越麻烦。
- ⚠️ **打包产物的写盘路径只按同一份代码推断**：设置页写盘是在 `target/release` 的 bin 上
  实测的，`.app` 那份实测到「启动 / 展开 / 托盘存在」，没有在 bundle 里点一遍设置。
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

1. ~~配置持久化~~ ✅ 2026-09-18 做完（`6a55bd5`）。
2. ~~macOS 真机验证~~ ✅ 2026-09-18 做完；`.dmg` 想彻底收尾可以真挂载装一遍。
3. **换掉 identifier**：`com.quotapanel.app` 去掉 `.app` 后缀，同时迁 `config.json`
   的位置（两份 README 的「配置」一节要跟着改）。
4. **作者邮箱**（见上面的 ⚠️）：换掉公开提交里的真实 Gmail 需要 Jules 亲自改写历史。

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
