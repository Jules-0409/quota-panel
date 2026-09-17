// 一次性脚本：为 cursor.rs 的 parse_iso_to_unix 生成期望值（epoch 秒）。
// 手算月份/闰年容易出错，直接用 Date.parse 对照。
const cases = [
  '2026-09-17T12:34:56Z',
  '1970-01-01T00:00:00Z',
  '2000-02-29T00:00:00Z', // 闰年
  '2026-12-31T23:59:59Z',
  '2026-09-17T12:34:56.789Z', // 带毫秒
];
for (const c of cases) {
  console.log(c + ' -> ' + Math.floor(Date.parse(c) / 1000));
}
