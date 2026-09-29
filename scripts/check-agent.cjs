// 账号页面注入脚本（adapter.rs 的 agent_js）的跨帧协议检查：node scripts/check-agent.cjs
//
// 每一帧是一个独立的 vm 上下文，有自己的 origin；postMessage 异步投递，source / origin /
// targetOrigin 的行为跟浏览器一致。真实的 agent_js 原样跑在每一帧里。
// 钉住的是 WebKit 里没法自动化测、错了又会直接漏密码的那几条：别家 iframe 抢答、
// 手动填落进旧输入框、伪造 apply、页面脚本冒充真人操作。
const vm = require("vm");
const fs = require("fs");
const path = require("path");
const rs = fs.readFileSync(path.join(__dirname, "../src-tauri/src/adapter.rs"), "utf8");
const start = rs.indexOf("pub fn agent_js()");
const AGENT = rs.slice(rs.indexOf("r#\"", start) + 3, rs.indexOf("\"#", start));

let current = null;
const runIn = (f, fn) => { const p = current; current = f; try { return fn(); } finally { current = p; } };
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

function makeFrame(name, origin, parent) {
  const f = { name, origin, fields: [], winL: {}, docL: {}, toasts: [], invokes: [], received: [], children: [] };
  const w = {};
  f.window = w;
  w.window = w;
  w.location = { origin };
  w.URL = URL;
  w.Date = Date;
  w.Object = Object;
  w.Array = Array;
  w.Event = class { constructor(t) { this.type = t; } };
  w.HTMLInputElement = function () {};
  Object.defineProperty(w.HTMLInputElement.prototype, "value", { set(v) { this._value = v; }, get() { return this._value; } });
  w.HTMLTextAreaElement = w.HTMLInputElement;
  w.setTimeout = (fn, ms) => setTimeout(() => runIn(f, fn), ms);
  w.clearTimeout = clearTimeout;
  w.setInterval = (fn, ms) => setInterval(() => runIn(f, fn), ms);
  w.clearInterval = clearInterval;
  w.addEventListener = (t, fn) => { (f.winL[t] ||= []).push(fn); };
  w.postMessage = (msg, targetOrigin) => {
    const src = current;
    const data = structuredClone(msg);
    setTimeout(() => {
      if (targetOrigin !== "*" && targetOrigin !== f.origin) return; // browser drops it
      f.received.push({ from: src.name, data });
      runIn(f, () => (f.winL.message || []).forEach((fn) => fn({ data, origin: src.origin, source: src.proxy })));
    }, 1);
  };
  w.__TAURI_INTERNALS__ = { invoke: (cmd) => { f.invokes.push(cmd); return Promise.resolve(); } };
  w.document = {
    addEventListener: (t, fn) => { (f.docL[t] ||= []).push(fn); },
    querySelectorAll: (sel) => f.fields.filter((x) => sel === "*" || sel === "#" + x.id),
    body: { appendChild: (d) => f.toasts.push(d.textContent) },
    createElement: () => ({ style: {}, remove() {}, textContent: "" }),
  };
  w.length = 0;
  vm.createContext(w);
  // inside a vm context `window` is the context's global proxy, not `w` — use the proxy as the
  // frame's identity everywhere (e.source, window.top, window[i]) like one WindowProxy in a browser
  f.proxy = vm.runInContext("globalThis", w);
  f.topProxy = parent ? parent.topProxy : f.proxy;
  w.top = f.topProxy;
  if (parent) { parent.children.push(f); parent.window[parent.window.length] = f.proxy; parent.window.length++; }
  runIn(f, () => vm.runInContext(AGENT, w));
  return f;
}

function field(f, id) {
  const el = {
    id, tagName: "INPUT", _value: "", isConnected: true, isContentEditable: false, style: {}, shadowRoot: null,
    getClientRects: () => [1],
    focus() { runIn(f, () => (f.docL.focusin || []).forEach((fn) => fn({ target: el, composedPath: () => [el] }))); },
    blur() {}, dispatchEvent: () => true,
  };
  f.fields.push(el);
  return el;
}

// a real user click on el: trusted window mousedown + document focusin + click
function userClick(f, el) {
  runIn(f, () => {
    (f.winL.mousedown || []).forEach((fn) => fn({ isTrusted: true }));
    (f.docL.focusin || []).forEach((fn) => fn({ target: el, composedPath: () => [el] }));
    (f.docL.click || []).forEach((fn) => fn({ target: el, composedPath: () => [el], preventDefault() {}, stopPropagation() {} }));
  });
}

// hostile page script inside frame f: answers every claim instantly to steal the values
function hostile(f) {
  f.window.addEventListener("message", (e) => {
    if (e.data && e.data.t === "claim") runIn(f, () => e.source.postMessage({ __sb: 1, t: "claimed", id: e.data.id, at: 9e15 }, "*"));
  });
}
const stole = (f) => f.received.some((r) => r.data.t === "apply");

const results = [];
const check = (name, ok) => results.push([ok ? "PASS" : "FAIL", name]);

(async () => {
  // S1: aliyun shape — form lives in a same-site cross-origin iframe
  {
    const top = makeFrame("top", "https://signin.aliyun.com", null);
    const pass = makeFrame("passport", "https://passport.aliyun.com", top);
    const u = field(pass, "u"), p = field(pass, "p");
    runIn(top, () => top.window.__sb.autofill("#u", "#p", "alice", "s3cret", true));
    await sleep(400);
    check("S1 autofill reaches same-site iframe (aliyun shape)", u._value === "alice" && p._value === "s3cret");
  }
  // S2: hostile cross-site iframe on the login page races to claim (and is earlier in frame order)
  {
    const top = makeFrame("top", "https://signin.aliyun.com", null);
    const evil = makeFrame("ads", "https://ads.evil.com", top);
    const pass = makeFrame("passport", "https://passport.aliyun.com", top);
    hostile(evil);
    const u = field(pass, "u"), p = field(pass, "p");
    runIn(top, () => top.window.__sb.autofill("#u", "#p", "alice", "s3cret", true));
    await sleep(400);
    check("S2 cross-site iframe cannot claim the password", !stole(evil));
    check("S2 honest iframe still gets filled", p._value === "s3cret");
  }
  // S3: stale focus — clicked a search box in the page first, then the password box in an iframe
  {
    const top = makeFrame("top", "https://www.example.com", null);
    const search = field(top, "search");
    const login = makeFrame("login", "https://login.example.com", top);
    const pw = field(login, "pw");
    userClick(top, search);
    await sleep(20);
    userClick(login, pw);
    runIn(top, () => top.window.__sb.fill("s3cret", "密码"));
    await sleep(400);
    check("S3 manual fill goes to the most recently clicked field", pw._value === "s3cret");
    check("S3 password does not land in the stale search box", search._value !== "s3cret");
  }
  // S4: plain single-frame manual fill still works
  {
    const top = makeFrame("top", "http://1.2.3.4:8888", null);
    const a = field(top, "a");
    userClick(top, a);
    runIn(top, () => top.window.__sb.fill("root", "账号"));
    await sleep(400);
    check("S4 single-frame manual fill (IP host)", a._value === "root");
  }
  // S5: sandboxed iframe ('null' origin) can't claim
  {
    const top = makeFrame("top", "https://signin.aliyun.com", null);
    const sb = makeFrame("sandbox", "null", top);
    hostile(sb);
    runIn(top, () => top.window.__sb.autofill("#u", "#p", "alice", "s3cret", true));
    await sleep(400);
    check("S5 null-origin iframe cannot claim", !stole(sb));
  }
  // S6: heartbeat — trusted input from a child frame reaches IPC once (throttled); untrusted never
  {
    const top = makeFrame("top", "https://console.aliyun.com", null);
    const child = makeFrame("child", "https://passport.aliyun.com", top);
    runIn(child, () => (child.winL.keydown || []).forEach((fn) => fn({ isTrusted: false })));
    await sleep(20);
    const afterFake = top.invokes.filter((c) => c === "touch_activity").length;
    runIn(child, () => (child.winL.keydown || []).forEach((fn) => fn({ isTrusted: true })));
    runIn(child, () => (child.winL.wheel || []).forEach((fn) => fn({ isTrusted: true })));
    await sleep(20);
    const afterReal = top.invokes.filter((c) => c === "touch_activity").length;
    const noScroll = !(child.winL.scroll || []).length && !(child.docL.scroll || []).length;
    check("S6 script-dispatched events don't count as activity", afterFake === 0);
    check("S6 real input pings once (throttled)", afterReal === 1);
    check("S6 scroll is not a heartbeat source", noScroll);
  }
  // S7: a sibling frame can't forge top's 'apply' into a waiting login field
  {
    const top = makeFrame("top", "https://signin.aliyun.com", null);
    const pass = makeFrame("passport", "https://passport.aliyun.com", top);
    const evil = makeFrame("ads", "https://ads.evil.com", top);
    const u = field(pass, "u");
    userClick(pass, u);
    runIn(evil, () => pass.window.postMessage({ __sb: 1, t: "claim", kind: "focused", id: 1 }, "*"));
    await sleep(20);
    runIn(evil, () => pass.window.postMessage({ __sb: 1, t: "apply", values: ["attacker@evil"], what: "x", change: true }, "*"));
    await sleep(20);
    check("S7 forged claim/apply from a sibling frame is ignored", u._value !== "attacker@evil");
  }

  // S8: the common shape — login form in the main document (宝塔 / 即构 / 云账户)
  {
    const top = makeFrame("top", "https://console.zego.im", null);
    const u = field(top, "u"), p = field(top, "p");
    runIn(top, () => top.window.__sb.autofill("#u", "#p", "bob", "pw2", true));
    await sleep(400);
    check("S8 autofill into main-document form", u._value === "bob" && p._value === "pw2");
  }
  // S9: recency the other way — iframe field first, then a main-document field: the main doc wins
  {
    const top = makeFrame("top", "https://www.example.com", null);
    const q = field(top, "q");
    const login = makeFrame("login", "https://login.example.com", top);
    const pw = field(login, "pw");
    userClick(login, pw);
    await sleep(20);
    userClick(top, q);
    runIn(top, () => top.window.__sb.fill("hello", "账号"));
    await sleep(400);
    check("S9 most recent wins even when it's the main document", q._value === "hello" && pw._value !== "hello");
  }

  for (const [s, n] of results) console.log(s, n);
  const ok = results.every(([s]) => s === "PASS");
  console.log(ok ? "ALL PASS" : "SOME FAILED");
  process.exit(ok ? 0 : 1);
})();
