// 每个文档各有一个代理。定位阶段不带值；填写只沿已确认的 frame 链传递。
(function () {
  'use strict';
  if (window.__sbBindings) return;
  const CHANNEL = '__switchboardBindingsV1';
  const top = window === window.top;
  const token = () => crypto.randomUUID();
  let documentId = token(), href = location.href, task = null, picking = null;
  let pickTimer = 0, hint = null;
  const page = () => {
    const u = new URL(location.href), queryEquals = {};
    // 一次性授权参数既不能稳定匹配，也不应进入绑定配置。
    const keep = /^(client_id|tenant|tenant_id|realm|app|app_id|mode|action|type|tab|lang)$/;
    for (const [k, v] of u.searchParams) { if (keep.test(k) && v.length <= 256) queryEquals[k] = v; }
    if (u.hash.startsWith('#/') && u.hash.includes('?')) {
      for (const [k, v] of new URLSearchParams(u.hash.slice(u.hash.indexOf('?') + 1))) {
        if (keep.test(k) && v.length <= 256) queryEquals['hash:' + k] = v;
      }
    }
    return { origin: u.origin, pathname: u.pathname, hashRoute: u.hash.startsWith('#/') ? u.hash.slice(1).split('?')[0] : null, queryEquals };
  };
  const matches = (expected, actual) => expected && expected.origin === actual.origin && expected.pathname === actual.pathname
    && expected.hashRoute === actual.hashRoute && Object.keys(expected.queryEquals).length === Object.keys(actual.queryEquals).length && Object.entries(expected.queryEquals).every(([k, v]) => actual.queryEquals[k] === v);
  const report = event => {
    if (top && window.__TAURI_INTERNALS__) return window.__TAURI_INTERNALS__.invoke('report_binding_event', { event }).catch(() => {});
  };
  function sameSite(a, b) {
    try {
      a = new URL(a); b = new URL(b);
      if (a.protocol !== b.protocol || !['http:', 'https:'].includes(a.protocol)) return false;
      const host = u => u.hostname.replace(/\.$/, '').toLowerCase();
      const site = h => { const p = h.split('.'); return p.slice(p.length > 2 && p[p.length - 1].length === 2 && /^(com|net|org|gov|edu|co|ac)$/.test(p[p.length - 2]) ? -3 : -2).join('.'); };
      if (!host(a).includes('.') || /^[\d.]+$/.test(host(a)) || host(a).includes(':')) return a.origin === b.origin;
      return site(host(a)) === site(host(b));
    } catch (_) { return false; }
  }
  function post(to, origin, payload) {
    if (origin && origin !== 'null') to.postMessage({ [CHANNEL]: true, ...payload }, origin);
  }
  function roots(root, fn, path = []) {
    for (const e of root.querySelectorAll('*')) {
      fn(e, path);
      if (e.shadowRoot) roots(e.shadowRoot, fn, [...path, { kind: 'shadow', locator: locator(e) }]);
    }
  }
  function frames() {
    const found = [];
    roots(document, (e, path) => {
      if (e.tagName === 'IFRAME') {
        try {
          const origin = new URL(e.src, location.href).origin;
          if (sameSite(location.origin, origin)) found.push({ e, path, origin });
        } catch (_) {}
      }
    });
    return found;
  }
  const css = value => CSS.escape(value);
  const quote = value => '"' + value.replace(/\\/g, '\\\\').replace(/"/g, '\\"').replace(/[\n\r\f]/g, c => '\\' + c.charCodeAt(0).toString(16) + ' ') + '"';
  function locator(el) {
    const root = el.getRootNode();
    const unique = value => { try { return root.querySelectorAll(value).length === 1; } catch (_) { return false; } };
    if (el.id && unique('#' + css(el.id))) return { kind: 'css', value: '#' + css(el.id) };
    const tag = el.tagName.toLowerCase();
    for (const key of ['data-testid', 'name', 'aria-label', 'placeholder']) {
      const value = el.getAttribute(key);
      if (value) {
        const selector = tag + '[' + key + '=' + quote(value) + ']';
        if (unique(selector)) return { kind: 'css', value: selector };
      }
    }
    const parts = [];
    for (let e = el; e && e.nodeType === 1; e = e.parentElement) {
      const name = e.tagName.toLowerCase();
      const siblings = e.parentNode ? Array.from(e.parentNode.children).filter(x => x.tagName === e.tagName) : [e];
      parts.unshift(name + ':nth-of-type(' + (siblings.indexOf(e) + 1) + ')');
      if (unique(parts.join(' > '))) return { kind: 'css', value: parts.join(' > ') };
    }
    throw new Error('unsupported_target');
  }
  function shadowPath(el) {
    const path = [];
    let root = el.getRootNode();
    while (root.host) {
      path.unshift({ kind: 'shadow', locator: locator(root.host) });
      root = root.host.getRootNode();
    }
    return path;
  }
  function editable(el, expected) {
    if (!el || !el.isConnected || el.disabled || el.readOnly || el.closest('[inert]')) return false;
    const tag = el.tagName.toLowerCase(), type = tag === 'input' ? el.type : '';
    if (!(tag === 'textarea' || (tag === 'input' && ['text', 'password', 'email', 'tel', 'url', 'search', 'number'].includes(type)))) return false;
    if (expected && (expected.tag !== tag || expected.type !== type)) return false;
    const style = getComputedStyle(el);
    return !!el.getClientRects().length && style.visibility !== 'hidden' && style.display !== 'none';
  }
  function unique(root, spec) {
    if (!spec || spec.kind !== 'css') throw new Error('unsupported_target');
    let items;
    try { items = root.querySelectorAll(spec.value); } catch (_) { throw new Error('unsupported_target'); }
    if (items.length > 1) throw new Error('ambiguous_target');
    return items[0] || null;
  }
  function resolve(target) {
    let root = document;
    for (let i = 0; i < target.contextPath.length; i++) {
      const step = target.contextPath[i], el = unique(root, step.locator);
      if (!el) return null;
      if (step.kind === 'shadow') {
        if (!el.shadowRoot) return null;
        root = el.shadowRoot;
      } else if (step.kind === 'frame') {
        if (el.tagName !== 'IFRAME' || !sameSite(location.origin, step.page.origin)) throw new Error('frame_mismatch');
        return { frame: el, page: step.page, target: { ...target, page: step.page, contextPath: target.contextPath.slice(i + 1) } };
      } else throw new Error('unsupported_target');
    }
    const el = unique(root, target.locator);
    if (!el) return null;
    if (!editable(el, target.expected)) throw new Error('unsupported_target');
    return { el };
  }
  function clearTask() {
    const old = task; task = null;
    if (!old) return;
    clearTimeout(old.timer); clearTimeout(old.verifyTimer);
    old.value = undefined;
    if (old.route) post(old.route.frame.contentWindow, old.route.page.origin, { op: 'cancel', runId: old.runId, stepId: old.stepId });
  }
  function stopPick() {
    picking = null; clearInterval(pickTimer); pickTimer = 0;
    if (hint) hint.remove(); hint = null;
    for (const f of frames()) post(f.e.contentWindow, f.origin, { op: 'stopPick' });
  }
  function reply(t, code, localDocument) {
    if (task !== t) return;
    const payload = { kind: 'reply', document: t.document, runId: t.runId, stepId: t.stepId, code, localDocument: localDocument || documentId };
    if (top) report({ kind: payload.kind, document: payload.document, runId: payload.runId, stepId: payload.stepId, code });
    else post(window.parent, t.parentOrigin, payload);
    if (code !== 'ready') clearTask();
  }
  function fresh(t) { return task === t && href === location.href && t.localDocument === documentId && matches(t.target.page, page()); }
  function locate(t) {
    if (task !== t) return;
    if (!fresh(t)) return reply(t, 'stale_document');
    try {
      const found = resolve(t.target);
      if (found && found.frame) {
        t.route = found;
        post(found.frame.contentWindow, found.page.origin, { op: 'locate', runId: t.runId, stepId: t.stepId, document: t.document, target: found.target, overwrite: t.overwrite });
        // 子 frame 可能还在加载，直到收到 ready 再停止重试。
      } else if (found) {
        t.el = found.el; t.ready = true;
        return reply(t, 'ready');
      }
    } catch (e) { return reply(t, e.message); }
    if (Date.now() - t.since >= 30000) return reply(t, 'target_not_found');
    t.timer = setTimeout(() => locate(t), 200);
  }
  function startLocate(m, parentOrigin) {
    if (task && task.runId === m.runId && task.stepId === m.stepId) {
      if (task.ready) reply(task, 'ready');
      return;
    }
    clearTask(); stopPick();
    task = { ...m, parentOrigin, localDocument: documentId, since: Date.now() };
    if (!matches(m.target.page, page())) return reply(task, 'page_mismatch');
    locate(task);
  }
  function apply(m) {
    const t = task;
    if (!t || !t.ready || t.runId !== m.runId || t.stepId !== m.stepId || t.document !== m.document) return;
    if (!fresh(t) || (m.localDocument && m.localDocument !== documentId)) return reply(t, 'stale_document');
    try {
      const found = resolve(t.target);
      if (t.route) {
        if (!found || found.frame !== t.route.frame) return reply(t, 'frame_mismatch');
        post(found.frame.contentWindow, found.page.origin, { ...m, localDocument: t.childDocument });
        return;
      }
      if (!found || found.el !== t.el) return reply(t, 'stale_document');
      if (t.el.value === m.value) return reply(t, 'already_matches');
      if (t.el.value && !t.overwrite) return reply(t, 'nonempty_skipped');
      t.value = m.value;
      const prototype = t.el.tagName === 'TEXTAREA' ? HTMLTextAreaElement.prototype : HTMLInputElement.prototype;
      Object.getOwnPropertyDescriptor(prototype, 'value').set.call(t.el, t.value);
      t.el.dispatchEvent(new Event('input', { bubbles: true, composed: true }));
      t.el.dispatchEvent(new Event('change', { bubbles: true, composed: true }));
      t.verifyTimer = setTimeout(() => {
        if (!fresh(t)) return reply(t, 'stale_document');
        reply(t, t.el.value === t.value ? 'filled' : 'value_rejected');
      }, 150);
    } catch (_) { reply(t, 'unsupported_target'); }
  }
  function pickBroadcast() {
    if (!picking) return;
    if (Date.now() > picking.until) return cancelPick();
    for (const f of frames()) post(f.e.contentWindow, f.origin, { op: 'pick', ...picking });
  }
  function startPick(m, parentOrigin) {
    if (picking && picking.pickId === m.pickId) return;
    clearTask(); stopPick();
    picking = { pickId: m.pickId, document: m.document, until: m.until || Date.now() + 60000, parentOrigin };
    if (top && document.body) {
      hint = document.createElement('div'); hint.textContent = '点击要绑定的输入框 · Esc 取消';
      Object.assign(hint.style, { position: 'fixed', top: '8px', left: '50%', transform: 'translateX(-50%)', zIndex: '2147483647', padding: '9px 16px', background: '#1d4ed8', color: '#fff', borderRadius: '8px', pointerEvents: 'none', font: '13px sans-serif' });
      document.body.appendChild(hint);
    }
    pickBroadcast(); pickTimer = setInterval(pickBroadcast, 500);
  }
  function picked(target) {
    const p = picking;
    if (!p || Date.now() > p.until) return;
    if (top) report({ kind: 'picked', document: p.document, pickId: p.pickId, target: { ...target, page: page() } });
    else post(window.parent, p.parentOrigin, { kind: 'picked', document: p.document, pickId: p.pickId, target, framePage: page() });
    stopPick();
  }
  document.addEventListener('click', e => {
    if (!picking || !e.isTrusted) return;
    const el = e.composedPath()[0];
    if (!el || !el.tagName || !editable(el)) return;
    e.preventDefault(); e.stopImmediatePropagation();
    try { picked({ contextPath: shadowPath(el), locator: locator(el), expected: { tag: el.tagName.toLowerCase(), type: el.tagName === 'INPUT' ? el.type : '' } }); } catch (_) {}
  }, true);
  function cancelPick() {
    if (!picking) return;
    const m = { kind: 'pickCancelled', document: picking.document, pickId: picking.pickId };
    if (top) report(m); else post(window.parent, picking.parentOrigin, m);
    stopPick();
  }
  document.addEventListener('keydown', e => { if (e.key === 'Escape' && picking) cancelPick(); }, true);
  window.addEventListener('message', e => {
    const m = e.data;
    if (!m || m[CHANNEL] !== true || !sameSite(location.origin, e.origin)) return;
    if (!top && e.source === window.parent) {
      if (m.op === 'pick') startPick(m, e.origin);
      else if (m.op === 'stopPick') stopPick();
      else if (m.op === 'locate') startLocate(m, e.origin);
      else if (m.op === 'apply') apply(m);
      else if (m.op === 'cancel' && (!m.runId || (task && task.runId === m.runId && task.stepId === m.stepId))) clearTask();
      return;
    }
    if (m.kind === 'pickCancelled' && picking && m.pickId === picking.pickId && m.document === picking.document && frames().some(f => f.e.contentWindow === e.source && f.origin === e.origin)) { cancelPick(); return; }
    if (m.kind === 'picked' && picking && m.pickId === picking.pickId && m.document === picking.document) {
      const f = frames().find(f => f.e.contentWindow === e.source && f.origin === e.origin);
      if (!f || !m.framePage || m.framePage.origin !== e.origin) return;
      picked({ ...m.target, contextPath: [...f.path, { kind: 'frame', locator: locator(f.e), page: m.framePage }, ...m.target.contextPath] });
    } else if (m.kind === 'reply' && task && task.route && e.source === task.route.frame.contentWindow
      && e.origin === task.route.page.origin && task.runId === m.runId && task.stepId === m.stepId && task.document === m.document) {
      if (m.code === 'ready') { clearTimeout(task.timer); task.ready = true; task.childDocument = m.localDocument; }
      reply(task, m.code); // 每一层回自己的文档代号，父级不需要孙级的代号。
    }
  });
  function refreshDocument() {
    if (href !== location.href) { clearTask(); stopPick(); documentId = token(); href = location.href; }
  }
  function hello() {
    refreshDocument();
    if (top && /^https?:$/.test(location.protocol)) report({ kind: 'document', document: documentId, page: page() });
  }
  window.__sbBindings = { hello, command(m) {
    if (!top || !m) return;
    refreshDocument();
    if (m.op === 'cancel') { clearTask(); stopPick(); return; }
    if (m.op === 'finish') { if (task && task.runId === m.runId) clearTask(); return; }
    if (m.document !== documentId) { hello(); return; }
    if (m.op === 'pick') startPick(m);
    else if (m.op === 'locate') startLocate(m);
    else if (m.op === 'apply') apply(m);
  } };
  window.addEventListener('pagehide', () => { clearTask(); stopPick(); });
  // SPA 路由不触发原生 on_page_load；同时覆盖首次初始化早于会话登记的情况。
  setInterval(hello, 1000);
  hello();
})();
