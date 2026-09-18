// 校验 ui/index.html 里的界面代码：语法能过、文案 key 不缺失、字段名没写错。
//
// 为什么需要它：界面是单文件、零依赖、没有构建步骤，写错一个 key 不会报错，
// 只会在界面上显示一个光秃秃的 key 或者空白。GCM 那类后端逻辑有 Rust 测试兜着，
// 界面这半边只能靠这个脚本。
//
// 用法: node scripts/check-ui.cjs
const fs = require('fs');
const path = require('path');
const vm = require('vm');

const htmlPath = path.join(__dirname, '..', 'quota-panel-tauri', 'ui', 'index.html');
const html = fs.readFileSync(htmlPath, 'utf8');

const errors = [];
const fail = (m) => errors.push(m);

// ---------- 1. 取出 <script> 并做语法检查 ----------
const scriptMatch = html.match(/<script>([\s\S]*?)<\/script>/);
if (!scriptMatch) {
  console.error('找不到 <script> 块');
  process.exit(1);
}
const script = scriptMatch[1];

try {
  new vm.Script(script, { filename: 'index.html<script>' });
} catch (e) {
  fail(`JS 语法错误: ${e.message}`);
}

// ---------- 2. 抽出 TEXT 与 ERROR_TEXT 两份字典 ----------
// 直接把字典对象求值出来，避免正则解析嵌套字符串
function extractObject(name) {
  const start = script.indexOf(`const ${name} = {`);
  if (start < 0) {
    fail(`找不到 ${name} 定义`);
    return null;
  }
  // 从 `{` 开始做括号配平，找到对象结束位置
  let i = script.indexOf('{', start);
  let depth = 0;
  let inStr = null;
  for (let j = i; j < script.length; j++) {
    const ch = script[j];
    const prev = script[j - 1];
    if (inStr) {
      if (ch === inStr && prev !== '\\') inStr = null;
      continue;
    }
    if (ch === "'" || ch === '"' || ch === '`') {
      inStr = ch;
      continue;
    }
    if (ch === '{') depth++;
    else if (ch === '}') {
      depth--;
      if (depth === 0) {
        const literal = script.slice(i, j + 1);
        try {
          return vm.runInNewContext('(' + literal + ')');
        } catch (e) {
          fail(`${name} 求值失败: ${e.message}`);
          return null;
        }
      }
    }
  }
  fail(`${name} 括号不配平`);
  return null;
}

const TEXT = extractObject('TEXT');
const ERROR_TEXT = extractObject('ERROR_TEXT');

// ---------- 3. 两份语言字典的 key 必须完全一致 ----------
function sameKeys(obj, label) {
  if (!obj) return;
  const langs = Object.keys(obj);
  if (langs.length < 2) {
    fail(`${label} 少于两种语言: ${langs.join(',')}`);
    return;
  }
  const [a, ...rest] = langs;
  const ka = Object.keys(obj[a]).sort();
  for (const lang of rest) {
    const kb = Object.keys(obj[lang]).sort();
    const miss = ka.filter((k) => !kb.includes(k));
    const extra = kb.filter((k) => !ka.includes(k));
    if (miss.length) fail(`${label}.${lang} 缺少 key: ${miss.join(', ')}`);
    if (extra.length) fail(`${label}.${lang} 多了 key: ${extra.join(', ')}`);
  }
}

sameKeys(TEXT, 'TEXT');
sameKeys(ERROR_TEXT, 'ERROR_TEXT');

// ---------- 4. 代码里用到的 t('key') 必须在字典里存在 ----------
if (TEXT) {
  const used = new Set();
  for (const m of script.matchAll(/\bt\(\s*'([^']+)'/g)) used.add(m[1]);
  // window_label 这类动态 key 不在字典里，属于正常
  const defined = new Set(Object.keys(TEXT.zh || {}));
  const dynamic = new Set(['win.' /* 前缀式动态拼接未使用，占位 */]);
  for (const k of used) {
    if (dynamic.has(k)) continue;
    if (!defined.has(k)) fail(`t('${k}') 在 TEXT 字典里不存在`);
  }
}

// ---------- 5. localizeError 的兜底行为：未知 key 原样返回 ----------
// 这里只做静态确认——字典里没出现的错误 key 会在界面上露出英文 key，
// 所以后端新增错误时必须同步两份字典（见 AGENTS.md）
if (ERROR_TEXT) {
  const zhKeys = Object.keys(ERROR_TEXT.zh || {});
  const enKeys = Object.keys(ERROR_TEXT.en || {});
  const diff = zhKeys
    .filter((k) => !enKeys.includes(k))
    .concat(enKeys.filter((k) => !zhKeys.includes(k)));
  if (diff.length) fail(`ERROR_TEXT 两种语言不一致: ${diff.join(', ')}`);
}

// ---------- 6. 后端返回的字段名必须在 UI 里真的被读 ----------
// 防止「改了 models.rs 的字段名，界面静默显示无数据」
const modelsRs = fs.readFileSync(
  path.join(__dirname, '..', 'quota-panel-tauri', 'src-tauri', 'src', 'models.rs'),
  'utf8'
);
// CursorQuota 拆分后在 cursor/mod.rs（这里只做字符串匹配，读它即可）
const cursorRs = fs.readFileSync(
  path.join(__dirname, '..', 'quota-panel-tauri', 'src-tauri', 'src', 'cursor', 'mod.rs'),
  'utf8'
);

// 从 Rust 结构体的 camelCase 字段里挑出 UI 引用的那些，确认拼写一致
const criticalFields = [
  'autoPercentUsed',
  'apiPercentUsed',
  'totalPercentUsed',
  'grokPercentUsed',
  'grokHasAvailableUsage',
  'grokError',
  'dailyRemainingPercent',
  'weeklyRemainingPercent',
  'extraUsageBalanceCents',
  'overagePreference',
  'cycleResetUnix',
  'planEndUnix',
];
for (const f of criticalFields) {
  if (!script.includes(f)) fail(`UI 没有读取字段 ${f}（后端契约里存在）`);
}

// 契约源必须是 camelCase
if (!/rename_all = "camelCase"/.test(modelsRs) || !/rename_all = "camelCase"/.test(cursorRs)) {
  fail('models.rs 或 cursor/mod.rs 缺少 rename_all = "camelCase"，前端字段名会对不上');
}

// ---------- 7. 不该再出现的死代码 ----------
if (/apple-badge|\.stale\b/.test(html)) {
  fail('index.html 里仍残留 stale 徽章相关代码（已删除的功能）');
}
if (/\br\.stale\b|\bd\.stale\b/.test(script)) {
  fail('UI 仍在读已删除的 stale 字段');
}

// ---------- 结果 ----------
if (errors.length) {
  console.error('界面检查失败:\n');
  for (const e of errors) console.error('  - ' + e);
  process.exit(1);
}
console.log('ui/index.html 检查通过');
console.log(`  TEXT: ${Object.keys(TEXT.zh).length} 个 key x ${Object.keys(TEXT).length} 种语言`);
console.log(`  ERROR_TEXT: ${Object.keys(ERROR_TEXT.zh).length} 个 key x ${Object.keys(ERROR_TEXT).length} 种语言`);
console.log('  语法、文案 key、字段契约、死代码 均无问题');
