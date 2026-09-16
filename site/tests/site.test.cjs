const { test } = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const root = path.resolve(__dirname, '..');
const html = fs.readFileSync(path.join(root, 'index.html'), 'utf8');
test('static page has working internal anchors and local assets', () => {
  const ids = [...html.matchAll(/\bid="([^"]+)"/g)].map(m => m[1]);
  assert.equal(ids.length, new Set(ids).size);
  for (const [, target] of html.matchAll(/href="#([^"]+)"/g)) assert.ok(ids.includes(target), target);
  for (const [, file] of html.matchAll(/(?:src|href)="(\/site-assets\/[^"#]+)"/g)) assert.ok(fs.existsSync(path.join(root, file)), file);
  assert.ok(!html.includes('onclick='));
  assert.equal((html.match(/<h1\b/g) || []).length, 1);
  assert.ok(html.includes('lang="zh-CN"'));
});
test('canonical public services stay on the short domain', () => {
  assert.ok(html.includes('curl -fsSL https://camofy.app/install.sh | sh'));
  assert.ok(html.includes('href="https://cloud.camofy.app/"'));
  assert.ok(!html.includes('mirror.camofy.app'));
  const edge = fs.readFileSync(path.join(root, 'edge.Caddyfile'), 'utf8');
  for (const endpoint of ['/sub/*', '/api/*', '/github/*', '/downloads/*', '/install.sh']) assert.ok(edge.includes(endpoint));
  assert.ok(edge.includes('redir https://cloud.camofy.app{uri} 308'));
  assert.ok(!edge.includes('handle_path'));
});
test('accessible controls and isolation policy remain explicit', () => {
  for (const value of ['aria-label="配置工作区示意"', 'aria-live="polite"', '<summary>', 'class="skip"']) assert.ok(html.includes(value));
  const css = fs.readFileSync(path.join(root, 'site-assets/site.css'), 'utf8');
  assert.ok(css.includes('focus-visible'));
  assert.ok(css.includes('prefers-reduced-motion'));
  const policy = fs.readFileSync(path.join(root, 'Caddyfile'), 'utf8');
  assert.ok(policy.includes("script-src 'self'"));
  assert.ok(!policy.includes('unsafe-inline'));
});
