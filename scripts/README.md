# scripts/

一次性或运维脚本。不是构建流程的一部分，平时用不到。

- `gen-gcm-vectors.cjs` —— 生成 `src-tauri/src/credentials/gcm.rs` 里 GCM 单元测试用的
  测试向量。Droid CLI 是 Node 写的、IV 是 16 字节，所以必须拿 Node 的 OpenSSL
  实现当对照，才能证明手搓的 GCM 和真实写出方一致。
- `gen-date-vectors.cjs` —— 生成 `src-tauri/src/cursor/time.rs` 里
  `parse_iso_to_unix` 测试的期望值（用 `Date.parse` 当对照，防手算闰年出错）。

两个脚本都是**只打印结果、不写文件**，改完把输出手抄回对应的 `#[test]`：

```bash
node scripts/gen-gcm-vectors.cjs
node scripts/gen-date-vectors.cjs
```

关键：`gen-gcm-vectors.cjs` 里的密钥必须是 `000102...1f` 这串 hex，
它和测试里 `fn key() -> Vec<u8> { (0..32u8).collect() }` 是同一把钥匙。
只改一处会让所有解密测试失败。
