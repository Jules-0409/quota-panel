// 生成 ui/index.html 的离线预览：注入一个假的 Tauri IPC 桥 + 一批覆盖各分支的假数据。
//
// 为什么需要它：界面是单文件、零依赖，没有构建步骤也没有测试框架。
// 透明窗口 + backdrop-filter 的问题光看代码看不出来，必须真的渲染一眼。
// 这个脚本不动真程序、不碰真凭据，产出的预览页放在 gitignore 的 tmp/ 下。
//
// 用法: node scripts/make-ui-preview.cjs
const fs = require('fs');
const path = require('path');

const root = path.join(__dirname, '..');
const src = path.join(root, 'quota-panel-tauri', 'ui', 'index.html');
const outDir = path.join(root, 'tmp');
fs.mkdirSync(outDir, { recursive: true });

let html = fs.readFileSync(src, 'utf8');

// 覆盖各个条件分支的假数据：正常值、null（无数据）、额度耗尽、以及一个读失败的源
const payload = {
  results: {
    factory: {
      ok: true,
      orgId: 'org_demo123',
      windows: {
        standard: {
          fiveHour: { usedPercent: 88, secondsRemaining: 3600 },
          weekly: { usedPercent: 45, secondsRemaining: 200000 },
          monthly: { usedPercent: 12, secondsRemaining: 2400000 },
        },
        core: {
          fiveHour: { usedPercent: 0, secondsRemaining: 3600 },
          weekly: { usedPercent: 100, secondsRemaining: 200000 },
          monthly: { usedPercent: 30, secondsRemaining: 2400000 },
        },
      },
      overagePreference: 'droidCore',
      extraUsageBalanceCents: 1234,
    },
    devin: {
      ok: true,
      planName: 'Pro',
      email: 'demo@example.com',
      teamId: 'team_demo',
      dailyRemainingPercent: 61,
      weeklyRemainingPercent: 72,
      dailyResetAtUnix: Math.floor(Date.now() / 1000) + 3600,
      weeklyResetAtUnix: Math.floor(Date.now() / 1000) + 300000,
      acuConsumed: 7,
      acuLimit: 20,
      planEndUnix: Math.floor(Date.now() / 1000) + 2500000,
      source: 'preview',
    },
    cursor: {
      ok: true,
      planName: 'pro',
      email: 'demo@example.com',
      autoPercentUsed: 59,
      apiPercentUsed: null,           // 分支：无数据
      totalPercentUsed: 54.4,
      grokPercentUsed: 63,
      grokHasAvailableUsage: false,   // 分支：额度耗尽 → 强制判红
      grokResetUnix: Math.floor(Date.now() / 1000) + 200000,
      cycleResetUnix: Math.floor(Date.now() / 1000) + 1800000,
    },
  },
  config: { refreshMinutes: 5, warnPercent: 70, dangerPercent: 90 },
  at: Date.now(),
};

const mock = `
<script>
(function () {
  const PAYLOAD = ${JSON.stringify(payload)};
  window.__PREVIEW__ = true;
  window.__TAURI__ = {
    core: {
      invoke: async (cmd) => {
        if (cmd === 'get_quota') return PAYLOAD;
        if (cmd === 'get_config') return PAYLOAD.config;
        return null;
      },
    },
    event: { listen: () => {} },
  };
})();
</script>
`;

// 注入点必须在界面主 <script> 之前（那个脚本第一行就读 window.__TAURI__）
const idx = html.indexOf('<script>');
if (idx < 0) throw new Error('找不到主 <script>，index.html 结构变了？');
html = html.slice(0, idx) + mock + html.slice(idx);

// 预览默认就是展开态，省得截图还要先点一下。
// 注意要精确匹配 HTML 属性：CSS 里也有 `.panel[data-mode="collapsed"]`，
// 粗暴 replace 会先改到样式表那一处，界面反而还是收起的。
html = html.replace(
  /(<div class="panel" id="panel" )data-mode="collapsed"/,
  '$1data-mode="expanded"'
);
if (!/id="panel" data-mode="expanded"/.test(html)) {
  throw new Error('没能把预览改成展开态，index.html 里 panel 的写法变了？');
}
// 让透明窗口在浏览器里也有底色，方便看对比度
html = html.replace(
  'html, body {',
  'html, body { background: #1b1b20 !important;'
);

const out = path.join(outDir, 'ui-preview.html');
fs.writeFileSync(out, html, 'utf8');
console.log('预览页已生成: ' + out);
