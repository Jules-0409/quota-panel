# PROGRESS.md — quota-panel

**这个文件是「做到哪了」的唯一权威。** 只看它 + `git log` 就应该能接手。
诚实优先：没验证的标 ⚠️，进度文件骗人比没有更糟。

最后更新：2026-09-17

---

## 当前状态

**可用，已推送到远端。** 三个数据源（Factory / Devin / Cursor 含 Grok Bot）在本机实测
都能取到数，`quota` CLI 和 GUI 共用同一套 fetcher。

- 工作树干净，`main` 与 `origin/main` 同步于 `1fe8c5d`。
- 2026-09-17 推送的那 4 个提交（`5bdbd7b`、`33e2f38`、`20491f4`、`1fe8c5d`）
  修掉了下面记录的 3 个 bug 并补上了测试。**`c424ac5` 及更早的版本没有这些修复。**
- 推送用的是当次明确授权（母版第二节）；以后每次推送仍要重新获得授权。

上一次实跑验证（2026-09-17，Windows 这台机器，真实登录态）：

- `cargo test --bins --lib` → **52 passed, 0 failed**
- `cargo clippy --all-targets -- -D warnings` → **通过，无 warning**
- `.\target\release\quota.exe`（真接口）→ 三个数据源都有数；
  `QUOTA_LANG=en` 与 `zh` 两种输出都验证过，无中英混排
- Factory `cycle` 类解析交叉验证：用真实 `cycleEnd` 对 Node 的 `Date.parse`，
  逐秒一致
- `node scripts/check-ui.cjs` → 通过（并做过反向测试：往副本里注入
  文案缺失、字段拼错、死代码，脚本都能报出来）
- 界面改动过眼睛：`scripts/make-ui-preview.cjs` 生成离线预览（假数据、不碰真凭据），
  截图存在 `docs/evidence/ui-preview.png`，三张卡、警告/危险配色、
  「无数据」和「额度已耗尽」两个分支都正常

## 提交记录

```
20491f4 Add project instructions, progress log, and UI/test tooling
33e2f38 Document the test suite in both READMEs
5bdbd7b Fix the ISO date and GCM IV parsing bugs, and add unit tests
c424ac5 Add a `quota` CLI sharing the GUI's fetchers
1fe59f9 Key the Cursor session cookie off the scoped auth id
2201bd0 Add an English UI that follows the system locale
12eee29 Add English README with a dedicated Devin data-path section
0ec90dd Stop periodic DOM rebuilds that flicker the transparent window
26afc18 Initial open-source release of Quota Panel
```

前三个是本轮的，连同 `1fe8c5d` 已在 2026-09-17 推到 `origin/main`。

## 最近做完的（2026-09-17 这一轮）

起因：给项目补工程规范（母版第五节「正式项目」档），顺手加测试，
**结果测试抓到 3 个真 bug**。

### 抓到的 bug（都有回归测试钉住）

1. **日期解析算错半年**（`cursor.rs:parse_iso_to_unix`）
   `(153 * (m + 9) % 12 + 2) / 5` 里，Rust 的 `*` 和 `%` 同优先级且左结合，
   实际被解析成 `(153 * (m + 9)) % 12`，月份贡献全错（9 月算成 6）。
   影响：Cursor 账期重置时间、Grok 重置时间全部偏移约半年。
   **实测验证**：修前 `2026-10-08T02:36:39Z` 解出的 unix 与 Node 差半年，
   修后与 Node `Date.parse` 逐秒一致。
   修法：显式加括号 `153 * ((m + 9) % 12)`。

2. **GCM 解不开 ≥32 字节的 IV**（`credentials.rs:decrypt_factory_payload`）
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

- ⚠️ **macOS 侧没跑过**。这台是 Windows。`credentials.rs` 的 Keychain 分支、
  `cursor.rs` 的 `security` 回退、`lib.rs` 的 macOS 私有 API 窗口行为
  **都没在本机验证**。跨平台改动要特别小心。
- ⚠️ **`quota-panel-tauri` 主程序（GUI）本轮没重新实跑**，只跑了 CLI 和测试。
  界面改的是「删掉 stale 徽章」，风险低，但按母版要求
  「界面改动必须过眼睛」，**下次动 UI 前应先 `cargo run` 看一眼**。
- ⚠️ **Devin 的 protobuf 字段号是猜的**（见 `AGENTS.md` 第五节）。
  现在能对上，厂商改协议就会静默读错值。
- ⚠️ **Grok Bot 的分母未公开**，百分比只能看趋势（README 已如实写明）。
- ⚠️ **配置没持久化**：刷新间隔和阈值还写死在代码里。
- 仓库**没有 CI**（无 `.github/`）。测试目前靠手动跑。
- **没有 `tests/` 集成测试目录**：所有测试都是 `#[cfg(test)] mod tests` 内联单元测试。
  涉及的纯函数够用，但跨模块的取数流程没有端到端测试（依赖真登录态，CI 上也难做）。

## 下一步（还没做，供接手者选）

1. **加 CI**：GitHub Actions 跑 `cargo test` + `clippy -D warnings`。
   不需要真凭据（测试全是纯函数），所以能直接上 GitHub 托管 runner。
2. **配置持久化**：刷新间隔 / 阈值写进配置文件 + 一个设置界面，
   解决 README「已知限制」第一条。
3. **拆文件**：`credentials.rs`（677 行）和 `cursor.rs`（569 行）都超了 400 行 soft cap。
4. **macOS 验证**：在 Mac mini（见母版第十节）上构建并实跑，确认
   Keychain 分支、`.app` 打包、无边框窗口行为。
5. **清理**：`D:\code2\` 顶层曾散落 png / ps1 / zip，已按母版第七节归到
   `D:\code2\backup\quota-panel-artifacts\`。

## 常用命令

```powershell
cd D:\code2\quota-panel\quota-panel-tauri\src-tauri
& "$env:USERPROFILE\.cargo\bin\cargo.exe" test --bins --lib
& "$env:USERPROFILE\.cargo\bin\cargo.exe" clippy --all-targets -- -D warnings
& "$env:USERPROFILE\.cargo\bin\cargo.exe" build --release --bin quota
.\target\release\quota.exe              # 三个数据源
$env:QUOTA_LANG='en'; .\target\release\quota.exe --json
```

## 怎么验证「没坏」

本项目取数依赖本机登录态，CI 上跑不了真接口，所以判断标准是：

1. `cargo test` 全绿（纯函数逻辑没坏）
2. `quota.exe` 实跑一次，三个数据源都有数或至少报出可读的错误
   （凭据/网络问题的错误文案是语言无关 key，便于定位）
3. 改了 UI → 实际 `cargo run` 看一眼（透明窗口的坑靠日志发现不了）
