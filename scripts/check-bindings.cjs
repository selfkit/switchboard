// 功能级协议回归：运行真实注入脚本，模拟 DOM root、独立 frame、消息来源与虚拟时钟。
// 不依赖浏览器/生产账号，不提供测试编排或业务断言平台。
const assert = require('node:assert/strict');
const fs = require('node:fs');
const vm = require('node:vm');
const AGENT = fs.readFileSync(require('node:path').join(__dirname, '../src-tauri/src/binding-agent.js'), 'utf8');
let current, now = 0, serial = 0;
const timers = new Map();
const within = (f, fn) => { const old = current; current = f; try { return fn(); } finally { current = old; } };
function schedule(fn, ms, interval = 0) { const id = ++serial; timers.set(id, { fn, at: now + ms, interval, frame: current }); return id; }
function tick(ms) {
  const end = now + ms;
  for (let n = 0; n < 100000; n++) {
    const next = [...timers].filter(([, t]) => t.at <= end).sort((a, b) => a[1].at - b[1].at)[0];
    if (!next) { now = end; return; }
    const [id, t] = next; now = t.at;
    if (t.interval) t.at += t.interval; else timers.delete(id);
    within(t.frame, t.fn);
  }
  throw Error('timer loop');
}
function root(host) {
  return { host, children: [], querySelectorAll(selector) {
    if (selector === '*') return this.children;
    return this.children.filter(e => selector === '#' + e.id);
  } };
}
function node(f, id, tag = 'INPUT', r = f.window.document) {
  const el = { id, tagName: tag, type: 'text', _value: '', isConnected: true, style: {}, nodeType: 1,
    getRootNode: () => r, getAttribute: key => key === 'id' ? id : null, closest: () => null,
    getClientRects: () => el.hidden ? [] : [1], dispatchEvent: e => { if (el.onEvent) el.onEvent(e); return true; },
    parentNode: r, parentElement: null, remove() { r.children = r.children.filter(e => e !== el); } };
  Object.defineProperty(el, 'value', { get() { return this._value; }, set(v) { this._value = v; } });
  r.children.push(el); return el;
}
function frame(url = 'https://console.example.com/login', parent = null, id = 'frame', parentRoot) {
  const f = { reports: [], received: [], win: {}, doc: {}, parent };
  const w = f.window = { URL, URLSearchParams, location: new URL(url), CSS: { escape: x => x }, crypto: { randomUUID: () => 'uuid-' + (++serial) },
    Date: { now: () => now }, getComputedStyle: e => ({ display: e.hidden ? 'none' : 'block', visibility: 'visible' }),
    Event: class { constructor(type) { this.type = type; } }, HTMLInputElement: function () {},
    setTimeout: (fn, ms) => schedule(fn, ms), setInterval: (fn, ms) => schedule(fn, ms, ms), clearTimeout: id => timers.delete(id), clearInterval: id => timers.delete(id),
    addEventListener: (name, fn) => (f.win[name] ||= []).push(fn),
    __TAURI_INTERNALS__: { invoke: (cmd, args) => { assert.equal(cmd, 'report_binding_event'); f.reports.push(structuredClone(args.event)); return Promise.resolve(); } },
  };
  Object.defineProperty(w.HTMLInputElement.prototype, 'value', { set(v) { this._value = v; }, get() { return this._value; } });
  w.HTMLTextAreaElement = w.HTMLInputElement;
  w.document = Object.assign(root(), { addEventListener: (name, fn) => (f.doc[name] ||= []).push(fn),
    createElement: () => ({ style: {}, remove() {} }), body: { appendChild() {} } });
  w.window = w;
  vm.createContext(w); f.proxy = vm.runInContext('globalThis', w);
  w.top = parent ? parent.window.top : f.proxy; w.parent = parent ? parent.proxy : f.proxy;
  w.postMessage = (message, targetOrigin) => {
    const src = current, data = structuredClone(message);
    schedule(() => {
      if (targetOrigin !== w.location.origin) return;
      f.received.push(data);
      within(f, () => (f.win.message || []).forEach(fn => fn({ source: src.proxy, origin: src.window.location.origin, data })));
    }, 1);
  };
  if (parent) { const el = node(parent, id, 'IFRAME', parentRoot); el.src = url; el.contentWindow = f.proxy; f.element = el; }
  within(f, () => vm.runInContext(AGENT, w)); return f;
}
const page = f => ({ origin: f.window.location.origin, pathname: f.window.location.pathname, hashRoute: f.window.location.hash.startsWith('#/') ? f.window.location.hash.slice(1).split('?')[0] : null, queryEquals: {} });
const loc = id => ({ kind: 'css', value: '#' + id });
const target = (f, id, path = []) => ({ page: page(f), contextPath: path, locator: loc(id), expected: { tag: 'input', type: 'text' } });
const doc = f => f.reports.filter(e => e.kind === 'document').at(-1).document;
const command = (f, m) => within(f, () => f.window.__sbBindings.command(m));
const locate = (f, t, opts = {}) => { const m = { op: 'locate', runId: 'run', stepId: 'step', document: doc(f), target: t, overwrite: false, ...opts }; command(f, m); tick(20); return m; };
const apply = (f, m, value = 'secret') => { command(f, { op: 'apply', runId: m.runId, stepId: m.stepId, document: m.document, value }); tick(200); };
const code = f => f.reports.filter(e => e.kind === 'reply').at(-1)?.code;
const click = (f, el, trusted = true) => within(f, () => f.doc.click.forEach(fn => fn({ isTrusted: trusted, composedPath: () => [el], preventDefault() {}, stopImmediatePropagation() {} })));
function test(name, fn) { timers.clear(); now = 0; fn(); console.log('PASS', name); }

test('同源同 selector 的两个 iframe 只填写指定路径，值不广播', () => {
  const top = frame(), a = frame('https://console.example.com/form', top, 'a'), b = frame('https://console.example.com/form', top, 'b');
  const ea = node(a, 'u'), eb = node(b, 'u');
  const m = locate(top, target(top, 'u', [{ kind: 'frame', locator: loc('b'), page: page(b) }]));
  assert.equal(code(top), 'ready'); assert.equal(eb.value, '');
  apply(top, m); assert.equal(code(top), 'filled'); assert.equal(ea.value, ''); assert.equal(eb.value, 'secret');
  assert(!a.received.some(m => 'value' in m));
});
test('Shadow → 跨源同站 frame → Shadow 路径指认、保存后可复现', () => {
  const top = frame(), host = node(top, 'host', 'SECTION'); host.shadowRoot = root(host);
  const child = frame('https://login.example.com/form', top, 'login', host.shadowRoot);
  const inner = node(child, 'inner', 'SECTION'); inner.shadowRoot = root(inner);
  const input = node(child, 'tenant', 'INPUT', inner.shadowRoot);
  command(top, { op: 'pick', document: doc(top), pickId: 'pick' }); tick(10); click(child, input); tick(20);
  const picked = top.reports.find(e => e.kind === 'picked'); assert(picked);
  assert.deepEqual(picked.target.contextPath.map(s => s.kind), ['shadow', 'frame', 'shadow']);
  assert.equal(input.value, '', '指认不填写');
  apply(top, locate(top, picked.target), 'tenant-42'); assert.equal(input.value, 'tenant-42'); assert.equal(code(top), 'filled');
});
test('两个目标、多命中、隐藏和只读控件都不盲填', () => {
  const top = frame(); const a = node(top, 'same'), b = node(top, 'same');
  locate(top, target(top, 'same')); assert.equal(code(top), 'ambiguous_target'); assert.equal(a.value, '');
  b.remove(); a.readOnly = true; locate(top, target(top, 'same'), { stepId: 'two' }); assert.equal(code(top), 'unsupported_target');
  a.readOnly = false; a.hidden = true; locate(top, target(top, 'same'), { stepId: 'three' }); assert.equal(code(top), 'unsupported_target');
});
test('非空值默认跳过、主动覆盖、相同值回执', () => {
  const top = frame(), input = node(top, 'u'); input.value = 'typed';
  apply(top, locate(top, target(top, 'u'))); assert.equal(code(top), 'nonempty_skipped'); assert.equal(input.value, 'typed');
  apply(top, locate(top, target(top, 'u'), { stepId: 'two', overwrite: true })); assert.equal(code(top), 'filled');
  apply(top, locate(top, target(top, 'u'), { stepId: 'three' })); assert.equal(code(top), 'already_matches');
});
test('等待异步元素，无法找到时有界结束', () => {
  const top = frame(); const m = locate(top, target(top, 'late')); const input = node(top, 'late'); tick(220);
  assert.equal(code(top), 'ready'); apply(top, m); assert.equal(input.value, 'secret');
  locate(top, target(top, 'missing'), { stepId: 'missing' }); tick(30100); assert.equal(code(top), 'target_not_found');
});
test('框架清空值会报告拒绝；目标被替换不写新节点', () => {
  const top = frame(), input = node(top, 'u'); input.onEvent = () => { input.value = ''; };
  apply(top, locate(top, target(top, 'u'))); assert.equal(code(top), 'value_rejected');
  const m = locate(top, target(top, 'u'), { stepId: 'replacement' }); input.remove(); input.isConnected = false;
  const replacement = node(top, 'u'); apply(top, m); assert.equal(replacement.value, ''); assert.equal(code(top), 'stale_document');
});
test('SPA 导航使旧任务失效，旧 apply 不能在新 route 填写', () => {
  const top = frame(), input = node(top, 'u'); const m = locate(top, target(top, 'u'));
  top.window.location = new URL('https://console.example.com/change-password'); apply(top, m);
  assert.equal(input.value, ''); assert.notEqual(doc(top), m.document);
  locate(top, { ...target(top, 'u'), page: { ...page(top), pathname: '/login' } }, { stepId: 'new' }); assert.equal(code(top), 'page_mismatch');
});
test('取消清除待填任务，晚到的 apply 无效', () => {
  const top = frame(), child = frame('https://login.example.com/form', top, 'login'), input = node(child, 'u');
  const m = locate(top, target(top, 'u', [{ kind: 'frame', locator: loc('login'), page: page(child) }]));
  command(top, { op: 'cancel' }); tick(10); apply(top, m); assert.equal(input.value, '');
});
test('frame 跳转后不接受旧任务，即使父级持有旧引用', () => {
  const top = frame(), child = frame('https://login.example.com/form', top, 'login'), input = node(child, 'u');
  const m = locate(top, target(top, 'u', [{ kind: 'frame', locator: loc('login'), page: page(child) }]));
  child.window.location = new URL('https://login.example.com/change-password'); apply(top, m);
  assert.equal(input.value, ''); assert.equal(code(top), 'stale_document');
});
test('跨站 frame 与同站旁系 frame 伪造消息不能获取/写入值', () => {
  const top = frame(), child = frame('https://login.example.com/form', top, 'login'), evil = frame('https://ads.evil.test/', top, 'evil'), sibling = frame('https://console.example.com/ad', top, 'ad');
  const input = node(child, 'u'), path = [{ kind: 'frame', locator: loc('login'), page: page(child) }];
  const m = locate(top, target(top, 'u', path));
  within(sibling, () => child.proxy.postMessage({ __switchboardBindingsV1: true, ...m, op: 'apply', value: 'forged' }, child.window.location.origin)); tick(20);
  assert.equal(input.value, ''); apply(top, m); assert.equal(input.value, 'secret'); assert(!evil.received.some(m => 'value' in m));
  locate(top, target(top, 'u', [{ kind: 'frame', locator: loc('evil'), page: page(evil) }]), { stepId: 'evil' }); assert.equal(code(top), 'frame_mismatch');
});
test('不接受脚本伪造点击；子 frame 的 Esc 撤销整次指认', () => {
  const top = frame(), child = frame('https://login.example.com/form', top, 'login'), input = node(child, 'u');
  command(top, { op: 'pick', document: doc(top), pickId: 'pick' }); tick(10); click(child, input, false); tick(10);
  assert(!top.reports.some(e => e.kind === 'picked'));
  within(child, () => child.doc.keydown.forEach(fn => fn({ key: 'Escape' }))); tick(20);
  assert(top.reports.some(e => e.kind === 'pickCancelled')); tick(600); click(child, input); tick(10);
  assert(!top.reports.some(e => e.kind === 'picked'));
});
test('页面配置排除授权码/token，保留稳定查询项与 hash route', () => {
  const top = frame('https://console.example.com/login?code=secret&state=nonce&tenant=corp#/auth?token=secret&action=login');
  const p = top.reports[0].page;
  assert.deepEqual(p.queryEquals, { tenant: 'corp', 'hash:action': 'login' }); assert.equal(p.hashRoute, '/auth');
  assert(!JSON.stringify(p).includes('secret'));
});

test('新增 action 等页面查询条件不复用原绑定', () => {
  const top = frame(), input = node(top, 'u'), original = target(top, 'u');
  top.window.location = new URL('https://console.example.com/login?action=reset');
  within(top, () => top.window.__sbBindings.hello());
  locate(top, original); assert.equal(code(top), 'page_mismatch'); assert.equal(input.value, '');
});
console.log('ALL PASS');
