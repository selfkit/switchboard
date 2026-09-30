//! FillEngine：把账号密码注入登录页，以及"手动指认输入框"的取点脚本。
//!
//! 全部逻辑都在 [`agent_js`] 这一段常驻脚本里，**注入到页面的每一个 frame**。
//! Rust 侧的 `webview.eval()` 只能打到主文档，而国内云厂商的登录框基本都不在主文档里：
//! 阿里云是 `mini-login-embedder` 往 `#aliyun-login` 里塞一个跨域 iframe，腾讯云同理。
//! 脚本只认主文档的话，点击记不到、指认点不中、自动填充也找不到框——三个功能一起哑掉，
//! 而 Gitee 这种表单就在主文档里的站却好好的。
//!
//! 所以改成：每帧一个 agent，主文档那个兼任调度。取值/填值走 postMessage 握手：
//! 主文档广播"谁手里有要填的框"（**不带密码**），认领的那帧回话，主文档只把值
//! 定向发回给它、并钉死 targetOrigin。密码不会广播给页面上的其它 iframe。

use crate::models::PlatformConfig;

/// 常驻在**每个 frame** 里的 agent。三件事：
/// 1. 记住这一帧里用户最后点过的输入框（"填账号/填密码"靠它）；
/// 2. 指认模式下接管点击，把选择器回传；
/// 3. 按选择器自动填充。
///
/// 几个绕不过去的现实问题：
/// - 登录页大多异步渲染，选择器不会在 load 完成那一刻就命中 —— 所以轮询重试。
/// - React/Vue 受控组件会忽略对 `el.value` 的直接赋值（框架自己记着旧值），必须走原生
///   value setter 再补发 input/change，页面才认。macOS 的 WKWebView 上尤其致命。
/// - 扫码/账号双表单的站（云账户、微信公众平台）DOM 里同时放两套输入框，选择器会先命中
///   **隐藏**的那一份，填进去神不知鬼不觉地没效果。所以只挑可见的元素。
/// - Shadow DOM 里的事件 `target` 会被重定向成宿主元素，必须用 `composedPath()[0]`。
///
/// 改完这段跑 `pnpm check:agent`（scripts/check-agent.cjs）：多帧模拟，把这段脚本原样跑在
/// 每一帧里，钉住"别家 iframe 领不到密码""手动填不落进旧输入框"这几条 WebKit 里没法自动测的行为。
pub fn agent_js() -> &'static str {
    r#"(function () {
  if (window.__sbAgent) return;
  window.__sbAgent = true;

  var TOP = window.top === window;
  var last = null;      // 这一帧里用户最后点过的输入框
  var lastAt = 0;       // 点它的时间。好几帧都记着"最后点过的框"时，靠它挑最近的那帧
  var waiting = null;   // 已认领、正等主文档把值发下来的元素
  var picking = false, picked = [], badges = [], tip = null;
  var seq = 0, jobs = {}, pickTimer = 0;

  // Shadow DOM 里 e.target 是宿主元素，composedPath()[0] 才是真正被点的那个
  function hit(e) {
    var path = e.composedPath && e.composedPath();
    return path && path.length ? path[0] : e.target;
  }
  function isField(el) {
    return !!el && (el.tagName === 'INPUT' || el.tagName === 'TEXTAREA' || el.isContentEditable);
  }
  // 同一个选择器可能命中多个（扫码表单一份、账号表单一份），只要画在屏幕上的那个。
  // 直查不到就往 shadow root 里钻——组件库包出来的输入框都躲在里面。
  // ponytail: 钻的时候要遍历全部元素找 shadow host，只在直查失败时才走，够用了
  function queryDeep(root, sel) {
    var all;
    try { all = root.querySelectorAll(sel); } catch (e) { return null; }
    for (var i = 0; i < all.length; i++) {
      if (all[i].getClientRects().length > 0) return all[i];
    }
    var nodes = root.querySelectorAll('*');
    for (var j = 0; j < nodes.length; j++) {
      if (nodes[j].shadowRoot) {
        var deep = queryDeep(nodes[j].shadowRoot, sel);
        if (deep) return deep;
      }
    }
    return null;
  }
  // "host >>> input"：指认时自己生成的写法，>>> 右边的在左边那个元素的 shadow root 里
  function visible(sel) {
    if (!sel) return null;
    var hops = sel.split('>>>');
    var root = document, el = null;
    for (var i = 0; i < hops.length; i++) {
      el = queryDeep(root, hops[i].trim());
      if (!el) return null;
      root = el.shadowRoot || root;
    }
    return el;
  }
  function setVal(el, v, fireChange) {
    if (el.isContentEditable) {
      el.textContent = v;
      el.dispatchEvent(new Event('input', { bubbles: true }));
      return;
    }
    var proto = el.tagName === 'TEXTAREA' ? HTMLTextAreaElement.prototype : HTMLInputElement.prototype;
    Object.getOwnPropertyDescriptor(proto, 'value').set.call(el, v);
    el.dispatchEvent(new Event('input', { bubbles: true }));
    if (fireChange) el.dispatchEvent(new Event('change', { bubbles: true }));
  }
  // eval 是单向的，拿不回返回值，所以填没填上都在页面上飘一条
  function toast(msg, ok) {
    if (!document.body) return;
    var d = document.createElement('div');
    d.textContent = msg;
    d.style.cssText = 'position:fixed;z-index:2147483647;left:50%;top:16px;transform:translateX(-50%);' +
      'padding:8px 14px;border-radius:8px;font:500 13px/1.4 sans-serif;color:#fff;' +
      'background:' + (ok ? '#00A870' : '#E34D59');
    document.body.appendChild(d);
    setTimeout(function () { d.remove(); }, 2200);
  }

  // 广播只用来问话和切指认模式，**永远不带值**
  function broadcast(msg) {
    (function walk(w) {
      try { w.postMessage(msg, '*'); } catch (e) {}
      var n = 0;
      try { n = w.length; } catch (e) {}
      for (var i = 0; i < n; i++) { try { walk(w[i]); } catch (e) {} }
    })(window);
  }
  // 回话只发给问话的那一帧，并钉死它的 origin。
  // 'null' origin（sandbox iframe）钉不住只能退回 '*'，但带值的 apply 只发给过了 sameSite 的帧，
  // 'null' 过不了，所以退回 '*' 的只会是不带值的回话
  function reply(e, msg) {
    try { e.source.postMessage(msg, e.origin && e.origin !== 'null' ? e.origin : '*'); } catch (err) {}
  }
  // 近似"同一个站"：协议 + 最后两段域名（com.cn、co.uk 这类取三段）。
  // IP、IPv6 和单段主机名没有"站"的概念，返回空串，只认完全同源。跟 session.rs 的 fill_allowed 同一套规则。
  // ponytail: 没带公共后缀表，github.io 这种多租户后缀会被当成一个站；登录页真在那种域名上再换
  function site(origin) {
    var u;
    try { u = new URL(origin); } catch (err) { return ''; }
    var h = u.hostname, p = h.split('.'), n = p.length;
    if (n < 2 || h.charAt(0) === '[' || /^\d+$/.test(p[n - 1])) return '';
    var take = n > 2 && p[n - 1].length === 2 && /^(com|net|org|gov|edu|co|ac)$/.test(p[n - 2]) ? 3 : 2;
    return u.protocol + '//' + p.slice(n - take).join('.');
  }
  // 只有跟主文档同站的帧才能认领账号密码。广播是发给所有 iframe 的，
  // 不挡的话页面上随便一个广告/统计 iframe 抢着回一句"我有框"就能把密码领走
  function sameSite(origin) {
    if (origin !== 'null' && origin === location.origin) return true;
    var s = site(origin);
    return s !== '' && s === site(location.origin);
  }

  function pathIn(root, el) {
    if (el.id && root.querySelectorAll('#' + CSS.escape(el.id)).length === 1) {
      return '#' + CSS.escape(el.id);
    }
    var name = el.getAttribute('name');
    if (name) {
      var s = el.tagName.toLowerCase() + '[name="' + name + '"]';
      if (root.querySelectorAll(s).length === 1) return s;
    }
    // 兜底：从元素往上拼最短的 nth-of-type 路径
    var stop = root.body || root;
    var parts = [];
    var node = el;
    while (node && node.nodeType === 1 && node !== stop) {
      var part = node.tagName.toLowerCase();
      var parent = node.parentElement;
      if (parent) {
        var same = Array.prototype.filter.call(parent.children, function (c) {
          return c.tagName === node.tagName;
        });
        if (same.length > 1) part += ':nth-of-type(' + (same.indexOf(node) + 1) + ')';
      }
      parts.unshift(part);
      if (root.querySelectorAll(parts.join(' > ')).length === 1) break;
      node = parent;
    }
    return parts.join(' > ');
  }
  // shadow root 里的元素在外面选不着，拼成 "宿主 >>> 里面的路径"，visible() 认这个写法
  function selectorFor(el) {
    var root = el.getRootNode ? el.getRootNode() : document;
    if (root && root.host) return selectorFor(root.host) + ' >>> ' + pathIn(root, el);
    return pathIn(document, el);
  }

  function badge(el, n) {
    var r = el.getBoundingClientRect();
    var d = document.createElement('div');
    d.textContent = n;
    d.style.cssText = 'position:fixed;z-index:2147483647;width:22px;height:22px;border-radius:50%;' +
      'background:#0052D9;color:#fff;font:600 13px/22px sans-serif;text-align:center;pointer-events:none;' +
      'left:' + (r.right - 11) + 'px;top:' + (r.top - 11) + 'px;';
    document.body.appendChild(d);
    badges.push(d);
    el.style.outline = '2px solid #0052D9';
  }
  function clearPicks() {
    badges.forEach(function (b) { b.remove(); });
    badges = [];
    picked.forEach(function (el) { el.style.outline = ''; });
    picked = [];
  }
  function setPicking(on) {
    if (picking === on) return;   // 重复广播不能把已经标好的 1、2 抹掉
    picking = on;
    clearPicks();
    if (tip) { tip.remove(); tip = null; }
    // 提示条只在主文档挂一条，否则每个 iframe 都顶一条
    if (on && TOP && document.body) {
      tip = document.createElement('div');
      tip.textContent = '指认模式：先点"账号"输入框（标 1），再点"密码"输入框（标 2）——点完自动记住，没有"确定"要按';
      tip.style.cssText = 'position:fixed;z-index:2147483646;left:0;right:0;top:0;padding:10px 16px;' +
        'background:#0052D9;color:#fff;font:500 13px/1.5 sans-serif;text-align:center;';
      document.body.appendChild(tip);
    }
  }
  // 选择器不是秘密，子帧直接甩给主文档，由它走 IPC
  // 回传成败都要飘一条：指认没有"确定"按钮，页面上不说话用户根本不知道记没记下来
  function report(u, p) {
    if (!TOP) {
      try { window.top.postMessage({ __sb: 1, t: 'picked', u: u, p: p }, '*'); } catch (e) {}
      return;
    }
    try {
      var r = window.__TAURI_INTERNALS__.invoke('report_picked_selectors', { usernameSelector: u, passwordSelector: p });
      if (r && r.then) {
        r.then(function () { toast('这两个框记住了，下次打开自动填', true); },
               function (e) { toast('没记下来：' + e, false); });
      } else {
        toast('这两个框记住了，下次打开自动填', true);
      }
    } catch (e) {
      toast('没记下来：' + e, false);
    }
  }

  // 自动锁定的活动心跳：账号页面是独立的原生 webview，用户在里面点/打字
  // 不会冒泡到主窗口的 DOM，主窗口的空闲计时全靠这条信号才知道"这边其实还在用"。
  // 节流到 5 秒一次——不是每次按键都值得一次 IPC。
  var lastPing = 0;
  function reportActivity() {
    var now = Date.now();
    if (now - lastPing < 5000) return;
    lastPing = now;
    try { window.__TAURI_INTERNALS__.invoke('touch_activity'); } catch (e) {}
  }
  // 只认**真人**输入：isTrusted 为假的是页面脚本自己 dispatch 的。也不用 scroll/focusin——
  // 脚本滚动、el.focus() 触发的这两种照样是 trusted，自动滚动的日志页（Graylog 跟随模式）
  // 会让账号库永远锁不上。清单跟 App.tsx 主窗口那边一致。
  // 挂在 window 的捕获阶段，又比页面脚本先注册：页面自己 stopPropagation 也拦不住
  function onInput(e) {
    if (!e.isTrusted) return;
    if (TOP) { reportActivity(); return; }
    // 跨域 iframe 里的操作甩给主文档统一节流、统一走 IPC
    try { window.top.postMessage({ __sb: 1, t: 'activity' }, '*'); } catch (err) {}
  }
  ['mousedown', 'keydown', 'wheel', 'touchstart'].forEach(function (t) {
    window.addEventListener(t, onInput, { capture: true, passive: true });
  });

  function remember(el) {
    if (!isField(el)) return;
    last = el;
    lastAt = Date.now();
  }

  document.addEventListener('focusin', function (e) {
    remember(hit(e));
  }, true);

  document.addEventListener('click', function (e) {
    var el = hit(e);
    remember(el);
    if (!picking || !el || (el.tagName !== 'INPUT' && el.tagName !== 'TEXTAREA')) return;
    e.preventDefault();
    e.stopPropagation();
    if (picked.length >= 2) clearPicks();
    picked.push(el);
    badge(el, picked.length);
    if (picked.length === 2) report(selectorFor(picked[0]), selectorFor(picked[1]));
  }, true);

  window.addEventListener('message', function (e) {
    var m = e.data;
    if (!m || m.__sb !== 1) return;

    if (TOP && m.t === 'activity') { reportActivity(); return; }
    if (TOP && m.t === 'claimed') {
      var job = jobs[m.id];
      if (!job || !sameSite(e.origin)) return;
      // 手动填：候选先攒着，窗口关了再挑最近点过框的那帧（见 ask）
      if (job.kind === 'focused') { job.cands.push({ e: e, at: m.at || 0 }); return; }
      finish(m.id, e);                  // 自动填：先认领的那帧赢，晚到的忽略
      return;
    }
    if (TOP && m.t === 'picked') { if (sameSite(e.origin)) report(m.u, m.p); return; }
    if (TOP && m.t === 'find') { openFind(); return; }

    // 问话、发值、切指认只认主文档发来的：别的 iframe 冒充主文档塞一个 apply，
    // 就能往正在等值的登录框里填上它自己想要的账号
    if (e.source !== window.top) return;
    if (m.t === 'claim') {
      var els = null;
      if (m.kind === 'focused') {
        if (last && last.isConnected) els = [last];
      } else {
        var u = visible(m.u), p = visible(m.p);
        if (u && p) els = [u, p];
      }
      if (!els) return;
      waiting = els;
      reply(e, { __sb: 1, t: 'claimed', id: m.id, at: lastAt });
    } else if (m.t === 'apply') {
      var got = waiting;
      waiting = null;
      if (!got) return;
      for (var i = 0; i < got.length && i < m.values.length; i++) {
        got[i].focus();
        setVal(got[i], m.values[i], m.change);
      }
      // 两个框是自动填充，填完让页面自己校验；单个框是手动填，光标留在那
      if (got.length > 1) got.forEach(function (el) { el.blur(); });
      else got[0].focus();
      toast('已填入' + m.what, true);
    } else if (m.t === 'pick') {
      setPicking(m.on);
    }
  });

  if (!TOP) return;

  function finish(id, e) {
    var job = jobs[id];
    delete jobs[id];
    clearTimeout(job.timer);
    reply(e, { __sb: 1, t: 'apply', values: job.values, what: job.what, change: job.change });
    if (job.onHit) job.onHit();
  }

  // 主文档兼任调度：问一圈谁手里有框，谁认领就把值定向发给谁
  function ask(claim, values, what, change, onHit, onMiss) {
    var id = ++seq;
    var job = { kind: claim.kind, cands: [], values: values, what: what, change: change, onHit: onHit };
    jobs[id] = job;
    claim.__sb = 1;
    claim.t = 'claim';
    claim.id = id;
    broadcast(claim);
    job.timer = setTimeout(function () {
      if (!jobs[id]) return;
      // 手动填：每一帧都记着自己"最后点过的框"而且不会忘。点过页头搜索框、再去 iframe
      // 里点密码框的话两帧都会认领，先到先得几乎总是主文档那个旧框赢——密码就明文进了搜索框。
      // 所以等齐了，给最近点过框的那帧
      if (job.cands.length) {
        var best = job.cands[0];
        for (var i = 1; i < job.cands.length; i++) if (job.cands[i].at > best.at) best = job.cands[i];
        finish(id, best.e);
        return;
      }
      delete jobs[id];
      if (onMiss) onMiss();
    }, 200);
  }

  // 页内查找：⌘F / Ctrl+F，或会话页顶上的「查找」按钮。查找框只画在主文档，子帧里按 ⌘F 转给主文档。
  // 高亮走 CSS Custom Highlight API：不改页面 DOM、不动选区，查找框能一直留着焦点接着打字。
  // ponytail: 只搜主文档，跨域 iframe 和页面自己 shadow DOM 里的字搜不到；一处匹配不能跨两个文本节点
  //   （"foo<b>bar</b>" 搜不到 foobar）；Safari 17.2 以前没有 Highlight API，只滚过去不高亮。真碰上了再按帧广播
  var FIND_MAX = 1000;
  var finder = null, ranges = [], cur = 0, findTimer = 0;
  function openFind() {
    if (!TOP) { try { window.top.postMessage({ __sb: 1, t: 'find' }, '*'); } catch (e) {} return; }
    if (!finder && !(finder = buildFinder())) return;
    finder.host.style.display = 'block';
    finder.input.focus();
    finder.input.select();
    if (finder.input.value) step(0);
  }
  function closeFind() {
    if (!finder) return;
    clearTimeout(findTimer);
    finder.host.style.display = 'none';
    ranges = [];
    if (window.CSS && CSS.highlights) { CSS.highlights.delete('sb-find'); CSS.highlights.delete('sb-find-cur'); }
  }
  function buildFinder() {
    if (!document.body) return null;
    var host = document.createElement('div');
    host.style.cssText = 'all:initial;position:fixed;z-index:2147483647;top:10px;right:16px';
    // 关着的 shadow root：页面的 CSS 伤不到它，TreeWalker 也不会把查找框自己的字搜进去
    var root = host.attachShadow({ mode: 'closed' });
    var box = document.createElement('div');
    box.style.cssText = 'display:flex;align-items:center;gap:4px;padding:6px 8px;background:#fff;border:1px solid #DCDCDC;' +
      'border-radius:8px;box-shadow:0 4px 16px rgba(0,0,0,.15);font:13px/1.4 -apple-system,"PingFang SC","Microsoft YaHei",sans-serif;color:#333';
    var input = document.createElement('input');
    input.placeholder = '在页面中查找';
    input.style.cssText = 'width:180px;padding:4px 6px;border:1px solid #DCDCDC;border-radius:5px;outline:none;font:inherit;color:inherit;background:#fff';
    var count = document.createElement('span');
    count.style.cssText = 'min-width:52px;text-align:center;font-size:12px;color:#999';
    box.appendChild(input);
    box.appendChild(count);
    [['↑', '上一个（⇧↩）', function () { step(-1); }], ['↓', '下一个（↩）', function () { step(1); }], ['✕', '关闭（Esc）', closeFind]]
      .forEach(function (b) {
        var el = document.createElement('button');
        el.textContent = b[0];
        el.title = b[1];
        el.onclick = b[2];
        el.style.cssText = 'padding:2px 7px;border:0;border-radius:5px;background:transparent;font:inherit;color:#666;cursor:pointer';
        box.appendChild(el);
      });
    root.appendChild(box);
    input.addEventListener('input', function () {
      clearTimeout(findTimer);
      findTimer = setTimeout(function () { cur = 0; step(0); }, 150);
    });
    // 别让页面自己的快捷键接着处理查找框里的按键
    input.addEventListener('keydown', function (e) {
      e.stopPropagation();
      if (e.isComposing) return; // 拼音选词按的回车，不是"下一个"
      if (e.key === 'Enter') step(e.shiftKey ? -1 : 1);
      else if (e.key === 'Escape') closeFind();
    });
    try {
      var sheet = new CSSStyleSheet();
      sheet.replaceSync('::highlight(sb-find){background-color:#FFE58F}::highlight(sb-find-cur){background-color:#FF9C2E}');
      document.adoptedStyleSheets = document.adoptedStyleSheets.concat([sheet]);
    } catch (e) {}
    // 挂在 <html> 上而不是 body：SPA 重画 body 的时候不会把它一起冲掉
    document.documentElement.appendChild(host);
    return { host: host, input: input, count: count };
  }
  // 每次都重搜：SPA 的内容随时在变，攒着的 Range 会指向已经没了的节点
  function step(d) {
    var q = finder.input.value;
    ranges = [];
    if (q && document.body) {
      var re = new RegExp(q.replace(/[.*+?^${}()|[\]\\]/g, '\\$&'), 'gi');
      var walk = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT);
      for (var n = walk.nextNode(); n && ranges.length < FIND_MAX; n = walk.nextNode()) {
        var p = n.parentElement;
        if (!p || /^(SCRIPT|STYLE|NOSCRIPT|TEXTAREA)$/.test(p.tagName)) continue;
        re.lastIndex = 0;
        if (!re.test(n.data) || !p.getClientRects().length) continue; // 藏起来的不算
        re.lastIndex = 0;
        for (var m = re.exec(n.data); m && ranges.length < FIND_MAX; m = re.exec(n.data)) {
          var r = document.createRange();
          r.setStart(n, m.index);
          r.setEnd(n, m.index + m[0].length);
          ranges.push(r);
        }
      }
    }
    var total = ranges.length;
    cur = total ? (cur + d + total) % total : 0;
    finder.count.textContent = !q ? '' : total ? (cur + 1) + '/' + total + (total >= FIND_MAX ? '+' : '') : '无结果';
    finder.count.style.color = q && !total ? '#E34D59' : '#999';
    if (window.CSS && CSS.highlights && window.Highlight) {
      var all = new Highlight();
      ranges.forEach(function (x) { all.add(x); });
      CSS.highlights.set('sb-find', all);
      var one = new Highlight();
      if (total) one.add(ranges[cur]);
      one.priority = 1;
      CSS.highlights.set('sb-find-cur', one);
    }
    if (!total) return;
    // 先让所在元素进视野，再按匹配本身的位置补一刀：日志页那种一大块 <pre>，元素居中了匹配还在屏幕外
    var box = ranges[cur].getBoundingClientRect();
    if (box.top >= 0 && box.bottom <= window.innerHeight) return;
    ranges[cur].startContainer.parentElement.scrollIntoView({ block: 'center' });
    box = ranges[cur].getBoundingClientRect();
    if (box.top < 0 || box.bottom > window.innerHeight) window.scrollBy(0, box.top - window.innerHeight / 2);
  }
  // 冒泡阶段、页面没拦才接：页面自己的编辑器（宝塔的文件编辑器）有查找就用它的，跟浏览器一个规矩
  window.addEventListener('keydown', function (e) {
    if (!e.isTrusted || e.defaultPrevented || !(e.metaKey || e.ctrlKey) || String(e.key).toLowerCase() !== 'f') return;
    e.preventDefault();
    openFind();
  });

  window.__sb = {
    find: openFind,
    // 填到"用户最后点过的那个框"——不依赖任何选择器的兜底
    fill: function (value, what) {
      ask({ kind: 'focused' }, [value], what, true, null, function () {
        toast('先在页面上点一下要填的输入框，再点' + what, false);
      });
    },
    // 按平台配置自动填充。页面是异步渲染的，而且用户可能还停在扫码 tab，所以轮询 30 秒
    autofill: function (uSel, pSel, uVal, pVal, change) {
      var tries = 0;
      var timer = setInterval(function () {
        if (++tries > 120) { clearInterval(timer); return; }
        ask({ kind: 'pair', u: uSel, p: pSel }, [uVal, pVal], '账号密码', change, function () {
          clearInterval(timer);
        }, null);
      }, 250);
    },
    pick: function (on) {
      clearInterval(pickTimer);
      broadcast({ __sb: 1, t: 'pick', on: !!on });
      // 登录页的 iframe 常常是点开指认之后才挂上来的（扫码/账密切 tab 就重挂一次），
      // 隔一秒补一次，新来的帧才知道现在在指认
      if (on) pickTimer = setInterval(function () { broadcast({ __sb: 1, t: 'pick', on: true }); }, 1000);
    }
  };
})();"#
}

/// 按平台配置自动填充（页面加载完后调一次）。
pub fn fill_js(cfg: &PlatformConfig, username: &str, password: &str) -> String {
    let u_sel = json_str(&cfg.username_selector);
    let p_sel = json_str(&cfg.password_selector);
    let u_val = json_str(username);
    let p_val = json_str(password);
    let fire_change = cfg.trigger_event.contains("change");
    format!("window.__sb && window.__sb.autofill({u_sel}, {p_sel}, {u_val}, {p_val}, {fire_change});")
}

/// 把一个值填进「用户最后点过的那个输入框」。
pub fn fill_focused_js(value: &str, what: &str) -> String {
    let v = json_str(value);
    let what = json_str(what);
    format!("window.__sb && window.__sb.fill({v}, {what});")
}

/// 进入"手动指认输入框"模式：点第一个框标 1（账号），第二个标 2（密码），
/// 两个都点完就把选择器回传给 App。对应 EmbeddedRefill.dc.html。
pub fn picker_js() -> String {
    "window.__sb && window.__sb.pick(true);".to_string()
}

pub fn stop_picker_js() -> &'static str {
    "window.__sb && window.__sb.pick(false);"
}

/// 打开页内查找框（会话页的「查找」按钮）
pub fn find_js() -> &'static str {
    "window.__sb && window.__sb.find();"
}

/// 页面加载完后自报"这页到底是不是能用"，结果回传给 `report_page_state`。
///
/// 为什么非要问页面自己：wry **没有导航失败的回调**，后端只能靠超时猜。
/// 但服务器返回 4xx/5xx 时导航是**成功**的（宝塔限 IP 那台 69ms 就回了 404），
/// 超时判定永远不触发，用户就一直盯着 nginx 的错误页干瞪眼。
///
/// 两个信号，任一成立就算打不开：
/// - `responseStatus >= 400`：主文档的 HTTP 状态码（Safari 17.4+ / macOS 14.4+），1.2 秒时就报
/// - 正文一个字都没有、也没有任何可交互元素：真·白板
///
/// 白板**不能一眼定生死**：SPA 首屏常常只有一个转圈的空 div，接口慢的时候（保活在后台刷新时更常见）
/// 1.2 秒还没画出来，以前就这样被误杀成"打不开"，页面被藏起来、再也不复查。所以连着 10 秒都是白板才报；
/// 报了之后页面自己又画出来了，再报一次，后端把"打不开"撤掉、页面重新露出来。
/// 改了跑 `pnpm check:agent`，里面有这段的检查
pub fn page_probe_js() -> &'static str {
    r#"(function () {
  var start = Date.now();
  function state() {
    var status = 0;
    try {
      var nav = performance.getEntriesByType('navigation')[0];
      if (nav && typeof nav.responseStatus === 'number') status = nav.responseStatus;
    } catch (e) {}
    var text = document.body ? (document.body.innerText || '').trim() : '';
    // 图、svg、frameset 也算有东西：只摆一张图 / 一个动画的页面不是白板
    var interactive = !!document.querySelector('input, button, form, a[href], video, canvas, iframe, frame, img, svg');
    return { status: status, textLen: text.length, interactive: interactive };
  }
  function blank(s) { return s.status < 400 && s.textLen === 0 && !s.interactive; }
  function report(s) {
    try { window.__TAURI_INTERNALS__.invoke('report_page_state', s); } catch (e) {}
  }
  function probe() {
    var s = state();
    if (blank(s) && Date.now() - start < 10000) { setTimeout(probe, 1000); return; }
    report(s);
    if (!blank(s)) return;
    var mo = new MutationObserver(function () {
      var now = state();
      if (blank(now)) return;
      mo.disconnect();
      report(now);
    });
    mo.observe(document.documentElement, { childList: true, subtree: true, characterData: true });
  }
  setTimeout(probe, 1200);
})()"#
}

/// 把值转成 JS 字符串字面量。`serde_json` 不转义 `<`，虽然 `webview.eval()` 不走
/// HTML 解析、`</script>` 伤不到人，但用户密码里什么都可能有，这里一并封死。
fn json_str(s: &str) -> String {
    serde_json::to_string(s)
        .unwrap_or_else(|_| "\"\"".into())
        .replace('<', "\\u003C")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(trigger: &str) -> PlatformConfig {
        PlatformConfig {
            platform: "阿里云".into(),
            username_selector: "#username".into(),
            password_selector: "#password".into(),
            trigger_event: trigger.into(),
            verified: true,
            updated_at: String::new(),
        }
    }

    #[test]
    fn escapes_values_into_the_script() {
        // 密码里带引号/反斜杠/换行也不能把脚本拼坏
        let js = fill_js(&cfg("input+change"), "a\"b", "p'\\\n</script>");
        assert!(js.contains(r#""a\"b""#), "用户名必须被转义: {js}");
        assert!(!js.contains("</script>"), "危险字面量必须被转义: {js}");
        assert!(js.ends_with(", true);"), "change 事件跟配置走: {js}");
    }

    #[test]
    fn change_event_follows_config() {
        assert!(fill_js(&cfg("input"), "u", "p").ends_with(", false);"));
    }

    #[test]
    fn agent_runs_in_every_frame() {
        let js = agent_js();
        // 跨帧握手的三个关键点：广播问话、定向回值、子帧把选择器甩给主文档
        assert!(js.contains("window.top === window"));
        assert!(js.contains("composedPath"), "shadow DOM 里要取真正被点的元素");
        assert!(js.contains("t: 'claimed'"));
    }
}
