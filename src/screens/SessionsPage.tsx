import { useEffect, useMemo, useState } from "react";
import * as api from "../api";
import type { Account, SessionInfo } from "../api";
import { Button, C, Dialog, IconButton, MOD, R, copySecret, copyText } from "../ui";
import {
  IconChevronDown,
  IconChevronLeft,
  IconChevronRight,
  IconClose,
  IconCopy,
  IconExternal,
  IconPlus,
  IconReload,
  IconSearch,
} from "../icons";
import BindingPanel from "./BindingPanel";
import { Sidebar, SideNav, TABS_H, type Page } from "./Layout";

/**
 * 「会话」页：这里只画左边那条 224px 的账号栏。
 * 右边整块是后端挂在主窗口上的原生 webview（真实登录页），前端画不了也不用画。
 */
export default function SessionsPage({
  accounts,
  sessionCount,
  onGo,
  onLock,
  onOpen,
  onAdd,
  onEdit,
  onAccountsChanged,
  pickTarget,
  onPickHandled,
}: {
  accounts: Account[];
  sessionCount: number;
  onGo: (p: Page) => void;
  onLock: () => void;
  /** 交给 App 统一处理：被区域检测拦下时要弹提示框，不能静默失败 */
  onOpen: (a: Account) => Promise<void>;
  onAdd: () => void;
  onEdit: (a: Account) => void;
  /** 这一页改了账号库里的东西（设为直达页），让 App 重新拉列表 */
  onAccountsChanged: () => void;
  /** 从「平台适配配置」点过来要指认的账号，页面加载好就自动进指认模式 */
  pickTarget?: string | null;
  onPickHandled: () => void;
}) {
  const [sessions, setSessions] = useState<SessionInfo[]>([]);
  const [active, setActive] = useState("");
  const [busy, setBusy] = useState("");
  const [err, setErr] = useState("");
  const [tip, setTip] = useState("");
  const [query, setQuery] = useState("");
  const [onlyOnline, setOnlyOnline] = useState(false);
  const [showAll, setShowAll] = useState(false);
  /** 当前账号的「更多操作」展开没有。切账号就收起 */
  const [more, setMore] = useState(false);
  /** 页签条上的地址输入框。null = 收起 */
  const [newUrl, setNewUrl] = useState<string | null>(null);
  /** 关会话要二次确认：页面里没提交的表单会随页面一起没 */
  const [closing, setClosing] = useState<Account | null>(null);
  /** 清除登录态也不可逆（cookie 快照和本机数据一起删），同样先确认 */
  const [clearing, setClearing] = useState<Account | null>(null);

  useEffect(() => {
    // 进来就把账号页面露出来，离开这一页再藏回去，否则它会一直盖着别的界面
    api.setSessionsVisible(true).catch((e) => setErr(String(e)));
    const tick = async () => {
      try {
        setSessions(await api.listActiveSessions());
        setActive(await api.activeSession());
      } catch (e) {
        setErr(String(e));
      }
    };
    tick();
    const t = window.setInterval(tick, 1500);
    return () => {
      window.clearInterval(t);
      api.setSessionsVisible(false).catch(() => {});
    };
  }, []);

  // 指认没有"确定"按钮，点完第二个框就直接写库了。这里把后端的回执显示出来，
  // 否则侧栏一直挂着"已进入指认模式"，用户不知道到底成了没有
  useEffect(() => {
    const un = api.onSelectorsPicked((platform) => {
      setErr("");
      setTip(`已记住「${platform}」的账号框和密码框，平台适配配置已更新，下次打开这个平台自动填充`);
    });
    return () => {
      void un.then((f) => f());
    };
  }, []);

  // 开新页签失败要说话，否则用户看到的又是"点了链接没反应"
  useEffect(() => {
    const un = api.onTabOpenFailed((reason) => {
      setTip("");
      setErr(`新页签打不开：${reason}`);
    });
    return () => {
      void un.then((f) => f());
    };
  }, []);

  // 「平台适配配置」里点的指认：页面是异步加载的，早一秒调 set_pick_mode 后端就报"还在加载"，
  // 所以盯着轮询结果，等这个账号真的加载完了再开，用户那边看起来就是"自动进了指认模式"
  useEffect(() => {
    if (!pickTarget) return;
    const s = sessions.find((x) => x.account_id === pickTarget);
    if (!s) return;
    // 打不开/被断开就放弃，别把这个意图一直攒着——否则用户过会儿自己开这个账号时，
    // 指认模式会莫名其妙自己蹦出来
    if (s.cut || s.failed) return onPickHandled();
    if (!s.loaded) return;
    onPickHandled();
    void run(
      () => api.setPickMode(pickTarget, true),
      "已进入指认模式：到右边页面上先点账号框（标 1），再点密码框（标 2），点完自动写回平台配置",
    );
  }, [pickTarget, sessions]);

  const online = useMemo(() => new Set(sessions.map((s) => s.account_id)), [sessions]);
  const byId = useMemo(() => new Map(sessions.map((s) => [s.account_id, s])), [sessions]);
  /** 未打开 / 被区域拦截断开 / 加载中 / 打不开 / 正常 */
  const stateOf = (id: string): "off" | "cut" | "loading" | "failed" | "ok" => {
    const s = byId.get(id);
    if (!s) return "off";
    if (s.cut) return "cut"; // 断开优先于其它状态：它是人为的，原因跟加载失败完全不同
    if (s.failed) return "failed";
    return s.loaded ? "ok" : "loading";
  };
  const current = accounts.find((a) => a.id === active);
  useEffect(() => setMore(false), [active]);
  const fillTip = `先在右边页面上点一下要填的框，再点这里。值同时会复制到剪贴板，填不进去就 ${MOD}V`;
  // 焦点在主窗口（刚点过侧栏）时按 ⌘F 也要能查；焦点在页面里时页面自己的 agent 接
  useEffect(() => {
    if (!current) return;
    const onKey = (e: KeyboardEvent) => {
      if (!(e.metaKey || e.ctrlKey) || e.key.toLowerCase() !== "f") return;
      e.preventDefault();
      void run(() => api.findInPage(current.id));
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [current?.id]);
  const currentInfo = current ? byId.get(current.id) : undefined;
  const currentState = current ? stateOf(current.id) : "off";
  /** 所有已打开账号的页签总数。内存涨的时候先看这个数是不是在变大 */
  const tabCount = useMemo(() => sessions.reduce((n, s) => n + s.tabs.length, 0), [sessions]);
  /** 当前页签。失败页/换协议重试都看它 */
  const activeTab = currentInfo?.tabs[currentInfo.active_tab];
  /** 手动输入的地址按 https 开的就建议 http，反之亦然 */
  const altScheme: "http" | "https" = (activeTab?.url ?? "").startsWith("https:") ? "http" : "https";
  const ordered = useMemo(
    () => [...accounts.filter((a) => online.has(a.id)), ...accounts.filter((a) => !online.has(a.id))],
    [accounts, online],
  );
  const keyword = query.trim().toLocaleLowerCase();
  const matching = ordered.filter((a) => {
    if (onlyOnline && !online.has(a.id)) return false;
    return !keyword || [a.related_app, a.remark, a.username, a.owner_type, a.platform]
      .some((value) => value?.toLocaleLowerCase().includes(keyword));
  });
  // 已打开的账号始终可见；其余账号先显示六个，搜索时直接展示全部匹配项。
  const visible = keyword || onlyOnline || showAll
    ? matching
    : matching.filter((a) => online.has(a.id) || a.id === active)
      .concat(matching.filter((a) => !online.has(a.id) && a.id !== active).slice(0, 6));
  const hiddenCount = matching.length - visible.length;
  async function run(f: () => Promise<unknown>, okTip = "") {
    setErr("");
    setTip("");
    try {
      await f();
      if (okTip) setTip(okTip);
    } catch (e) {
      setErr(String(e));
    }
  }

  /**
   * 填一个字段，同时把值放进剪贴板。
   *
   * 注入永远有治不了的站：canvas 画的软键盘、原生控件、把 keydown 当唯一真相的输入框。
   * 与其让用户卡在那，不如每次都顺手复制一份——填上了当没这回事，没填上直接 ⌘V。
   */
  const fill = (id: string, field: Parameters<typeof api.fillFocused>[1], label: string) =>
    run(async () => {
      const value = await api.fillFocused(id, field);
      const copied = await copySecret(value);
      setTip(copied ? `已填${label}；没进去就直接在输入框里 ${MOD}V（已复制，30 秒后自动清空）` : `已填${label}`);
    });

  const keepAlive = (id: string, on: boolean, anyNetwork: boolean) =>
    run(async () => {
      await api.setKeepAlive(id, on, anyNetwork);
      setSessions(await api.listActiveSessions());
    });

  async function open(a: Account) {
    setBusy(a.id);
    await run(async () => {
      await onOpen(a);
      setActive(await api.activeSession());
      setSessions(await api.listActiveSessions());
    });
    setBusy("");
  }

  const toastStyle = { padding: "8px 10px", borderRadius: R.sm, fontSize: 12, lineHeight: 1.6 } as const;

  // 右边是原生 webview，永远盖在 HTML 之上；页内弹确认框时先把它藏起来，
  // 否则用户只能看到一层半透明的遮罩，对话框被页面挡掉。
  useEffect(() => {
    if (!closing && !clearing) return;
    void api.setSessionsVisible(false).catch(() => {});
    return () => {
      void api.setSessionsVisible(true).catch(() => {});
    };
  }, [closing, clearing]);

  const action = (label: string, onClick: () => void, title?: string) => (
    <button
      type="button"
      onClick={onClick}
      title={title}
      className="sb-btn sb-btn-sm sb-btn-ghost"
      style={{
        flexGrow: 1,
        minWidth: 0,
        overflow: "hidden",
        textOverflow: "ellipsis",
      }}
    >
      {label}
    </button>
  );

  /*
   * 当前账号的操作从账号列表里挪到侧栏底部固定区：
   * 以前它内联在列表里，展开/收起会改变列表长度和位置，切账号时按钮跟着跑。
   * 现在列表始终稳定，操作区固定高度、自己滚动。
   */
  const detailPanel = current && online.has(current.id) ? (
    <div style={{ display: "flex", flexDirection: "column", gap: 6 }}>
      <div style={{ fontSize: 11, color: C.muted, padding: "0 4px", overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
        当前账号 · {current.related_app || current.username}
      </div>
      {/* 页面没加载出来时填充无从谈起，与其摆一排点了就报错的按钮，不如直说 */}
      {currentState === "ok" ? (
        <div style={{ display: "flex", gap: 6 }}>
          {action("填账号", () => fill(current.id, "username", "账号"), fillTip)}
          {action("填密码", () => fill(current.id, "password", "密码"), fillTip)}
          {current.totp_secret && action("填验证码", () => fill(current.id, "totp", "验证码"), fillTip)}
        </div>
      ) : (
        <div style={{ fontSize: 12, color: C.muted, lineHeight: 1.6 }}>
          {currentState === "failed" ? "页面没打开，先刷新" : "页面加载中…"}
        </div>
      )}
      {currentState === "failed" && (
        <div style={{ display: "flex" }}>
          {action("刷新页面", () => run(() => api.reloadSession(current.id)), "重新加载这个页面")}
        </div>
      )}
      {currentInfo && (
        <label
          title="这个账号约 3 分钟没操作就自动刷新，没提交的表单会丢。锁屏和安全锁定后继续保活，直到关闭会话或取消勾选；期间阻止电脑空闲睡眠，仍可熄屏。合盖、主动睡眠、断网或网站强制过期仍可能需要重新登录。网络异常（区域检测判为要拦）时暂停"
          style={{ display: "flex", alignItems: "center", gap: 6, cursor: "pointer", fontSize: 12, color: C.sub }}
        >
          <input type="checkbox" checked={currentInfo.keep_alive} onChange={(e) => keepAlive(current.id, e.target.checked, false)} style={{ accentColor: C.brand, margin: 0 }} />
          保活（锁屏后继续）
        </label>
      )}
      <button
        type="button"
        onClick={() => setMore(!more)}
        className="sb-link"
        style={{ color: C.sub, fontSize: 12, padding: "2px 0", textAlign: "left", display: "inline-flex", alignItems: "center", gap: 4 }}
      >
        {more ? "收起操作" : "更多操作"}
        <span style={{ display: "inline-flex", transform: more ? "rotate(180deg)" : "none", transition: "transform 120ms ease" }}>
          <IconChevronDown size={12} />
        </span>
      </button>
      {more && (
        <>
          {/*
            绑定面板和保活细节都是低频操作，收进「更多」里。否则这块固定区会把
            账号列表挤到只剩一两行——列表才是切换账号的主入口。
          */}
          {currentInfo && !currentInfo.tabs[currentInfo.active_tab]?.external && (
            <BindingPanel key={currentInfo.window_label} account={current} tabLabel={currentInfo.window_label} ready={currentState === "ok"} />
          )}
          {currentInfo && (
            <div style={{ display: "flex", flexDirection: "column", gap: 4, fontSize: 12, color: C.sub }}>
              {currentInfo.keep_alive && (
                <label
                  title="你同意这个账号走境外网络 / 代理：区域检测判为网络异常时照常保活，区域拦截也不断开它。只给允许境外访问的账号勾"
                  style={{ display: "flex", alignItems: "center", gap: 6, cursor: "pointer", paddingLeft: 18 }}
                >
                  <input type="checkbox" checked={currentInfo.keep_alive_any_network} onChange={(e) => keepAlive(current.id, true, e.target.checked)} style={{ accentColor: C.brand, margin: 0 }} />
                  网络异常时也保活、不断开
                </label>
              )}
              {currentInfo.keep_alive && (
                <div
                  style={{
                    fontSize: 11,
                    color: currentInfo.keep_alive_error ? C.danger : C.muted,
                    lineHeight: 1.6,
                    paddingLeft: 18,
                    wordBreak: "break-all",
                  }}
                >
                  {currentInfo.keep_alive_error
                    ? `保活异常：${currentInfo.keep_alive_error}`
                    : currentInfo.keep_alive_last_secs != null
                      ? `上次保活 ${ago(currentInfo.keep_alive_last_secs)}，${untilRefresh(currentInfo.keep_alive_next_secs)}`
                      : `已开启，${untilRefresh(currentInfo.keep_alive_next_secs)}`}
                </div>
              )}
            </div>
          )}
          <div style={{ display: "grid", gridTemplateColumns: "repeat(2, minmax(0, 1fr))", gap: 6 }}>
            {currentState === "ok" && current.extra_fields.map((field, index) => field.value ? (
              <div key={index} style={{ minWidth: 0, display: "flex" }}>
                {action(`填${field.label}`, () => fill(current.id, `extra:${index}`, field.label), fillTip)}
              </div>
            ) : null)}
            {/* 刷新任何时候都得在：页面打不开的时候恰恰是最需要它的时候 */}
            {action("刷新页面", () => run(() => api.reloadSession(current.id)), "重新加载这个页面")}
            {action("编辑账号", () => onEdit(current), "修改当前账号及附加字段")}
            {/* 「设为直达页」会去问 WKWebView 要 URL，页面没加载出来时它是 nil，见 session.rs 的 url_is_safe */}
            {currentState === "ok" && action(
              "设为直达页",
              () =>
                run(async () => {
                  await api.pinCurrentUrl(current.id);
                  onAccountsChanged();
                }, "已把当前页面设为该账号的登录直达 URL"),
              "把右边现在停的这个地址存成该账号的登录直达 URL，下次直接开这里",
            )}
            {currentState === "ok" && action(
              "旧版平台指认",
              () =>
                run(
                  () => api.setPickMode(current.id, true),
                  "已进入指认模式：到右边页面上先点账号框（标 1），再点密码框（标 2），点完自动记住",
                ),
              "教一次这个平台的账号框和密码框在哪，之后自动填充",
            )}
            {/*
              在网站上退出登录后 cookie 可能全空，快照会跳过空值，旧快照就留着，
              下次打开又被塞回去"自动登录"。页面打不开时也可能是登录态坏了，所以一直显示
            */}
            {action(
              "清除登录状态",
              () => setClearing(current),
              "关掉页面，并删掉这个账号保存的 cookie 和本机登录数据，下次打开需要重新登录",
            )}
          </div>
        </>
      )}
    </div>
  ) : null;

  return (
    <>
    <Sidebar
      footer={
        <>
          {detailPanel && (
            <div
              style={{
                flexShrink: 0,
                maxHeight: "32vh",
                overflowY: "auto",
                padding: 8,
                margin: "8px 0 4px",
                background: C.bg,
                border: `1px solid ${C.border}`,
                borderRadius: 8,
                boxSizing: "border-box",
              }}
            >
              {detailPanel}
            </div>
          )}
          {/* 提示条钉在底部导航上方，不跟账号列表一起滚：账号一多它就被挤出可视区，
              右上角按钮点了、报错了，用户看到的却是"没反应" */}
          {(tip || err) && (
            <div
              role={err ? "alert" : "status"}
              aria-live={err ? "assertive" : "polite"}
              style={{
                ...toastStyle,
                display: "flex",
                alignItems: "flex-start",
                gap: 6,
                margin: "4px 0 8px",
                background: err ? C.dangerSoft : C.brandSoft,
                color: err ? C.danger : C.brand,
              }}
            >
              <span style={{ flex: 1, minWidth: 0 }}>{err || tip}</span>
              <IconButton label="关闭提示" size="xs" onClick={() => { setTip(""); setErr(""); }} style={{ marginTop: -2 }}>
                <IconClose size={12} />
              </IconButton>
            </div>
          )}
          <SideNav current="sessions" sessionCount={sessionCount} onGo={onGo} onLock={onLock} />
        </>
      }
    >
      <div style={{ display: "flex", flexDirection: "column", gap: 8 }}>
        <div style={{ fontSize: 12, color: C.muted, padding: "0 8px" }}>账号 · {online.size} 个已打开 / {accounts.length} 个</div>
        <div style={{ fontSize: 11, color: C.muted, padding: "0 8px", marginTop: -4 }}>共 {tabCount} 个页签</div>
        <Button kind="ghost" size="sm" onClick={onAdd}>+ 新增账号</Button>
        <div className="sb-search-wrap" style={{ width: "100%" }}>
          <input
            type="text"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder="搜索账号、归属或平台"
            aria-label="搜索会话账号"
            style={{ width: "100%", boxSizing: "border-box", height: 32, border: `1px solid ${C.border}`, borderRadius: R.sm, background: C.surface, padding: query ? "0 30px 0 9px" : "0 9px", fontSize: 12, color: C.text }}
          />
          {query && (
            <button type="button" className="sb-search-clear" aria-label="清除搜索" title="清除搜索" onClick={() => setQuery("")}>
              <IconClose size={12} />
            </button>
          )}
        </div>
        <div style={{ display: "flex", gap: 6 }}>
          {([false, true] as const).map((onlineOnly) => (
            <button
              key={String(onlineOnly)}
              type="button"
              aria-pressed={onlyOnline === onlineOnly}
              onClick={() => setOnlyOnline(onlineOnly)}
              style={{ flex: 1, border: `1px solid ${onlyOnline === onlineOnly ? C.brand : C.border}`, background: onlyOnline === onlineOnly ? C.brandSoft : C.surface, color: onlineOnly === onlineOnly ? C.brand : C.sub, borderRadius: R.sm, padding: "5px 0", fontSize: 12, cursor: "pointer" }}
            >
              {onlineOnly ? `已打开 ${online.size}` : `全部 ${accounts.length}`}
            </button>
          ))}
        </div>
      </div>

      <div style={{ display: "flex", flexDirection: "column", gap: 2 }}>
        {visible.map((a) => {
          const on = online.has(a.id);
          const cur = a.id === active;
          return (
            <div
              key={a.id}
              style={{
                display: "flex",
                alignItems: "center",
                gap: 2,
                borderRadius: "var(--sb-radius-sm)",
                background: cur ? C.bg : "transparent",
                border: `1px solid ${cur ? C.border : "transparent"}`,
              }}
            >
              <button
                type="button"
                onClick={() => open(a)}
                title={`${a.owner_type} · ${a.platform} · ${a.related_app || a.username}`}
                aria-current={cur ? "page" : undefined}
                className="sb-pressable"
                style={{
                  flexGrow: 1,
                  minWidth: 0,
                  display: "flex",
                  alignItems: "center",
                  gap: 8,
                  padding: "6px 8px",
                  border: 0,
                  borderRadius: "var(--sb-radius-sm)",
                  background: "transparent",
                  color: "inherit",
                  textAlign: "left",
                }}
              >
                <span aria-hidden="true" style={{ width: 6, height: 6, borderRadius: "var(--sb-radius-pill)", flexShrink: 0, background: on ? C.success : C.borderStrong }} />
                <span className="sb-sr-only">{on ? "已打开" : "未打开"}</span>
                <span style={{ display: "flex", flexDirection: "column", gap: 1, minWidth: 0, flexGrow: 1 }}>
                  <span style={{ fontSize: 12, color: cur ? C.text : C.sub, whiteSpace: "nowrap", overflow: "hidden", textOverflow: "ellipsis" }}>
                    {a.related_app || a.username}
                  </span>
                  <span style={{ fontSize: 12, color: C.muted, whiteSpace: "nowrap", overflow: "hidden", textOverflow: "ellipsis" }}>
                    {/* 归属放前面：同平台同名的两个账号，能分清的就是它 */}
                    <span style={{ color: C.sub }}>{a.owner_type}</span> · {a.platform}
                    {busy === a.id && " · 打开中…"}
                    {busy !== a.id && stateOf(a.id) === "loading" && " · 加载中…"}
                    {stateOf(a.id) === "failed" && <span style={{ color: C.danger }}> · 打不开</span>}
                    {stateOf(a.id) === "cut" && <span style={{ color: C.danger }}> · 已断开</span>}
                    {byId.get(a.id)?.keep_alive && " · 保活"}
                  </span>
                </span>
              </button>
              {on && (
                <IconButton
                  label="关闭这个账号"
                  title="关闭这个账号（会再确认）"
                  size="sm"
                  onClick={() => setClosing(a)}
                  style={{ marginRight: 2 }}
                >
                  <IconClose size={12} />
                </IconButton>
              )}
            </div>
          );
        })}
        {hiddenCount > 0 && (
          <button
            type="button"
            onClick={() => setShowAll(true)}
            className="sb-link" style={{ padding: "8px 10px", fontSize: 12 }}
          >
            显示其余 {hiddenCount} 个账号
          </button>
        )}
        {showAll && !keyword && !onlyOnline && matching.length > 6 && (
          <button
            type="button"
            onClick={() => setShowAll(false)}
            className="sb-link" style={{ padding: "8px 10px", fontSize: 12 }}
          >
            收起账号列表
          </button>
        )}
        {matching.length === 0 && (
          <div style={{ padding: "8px 10px", fontSize: 12, color: C.muted }}>
            {accounts.length === 0 ? "账号库是空的" : keyword ? "没有匹配的账号" : "暂无已打开的账号"}
          </div>
        )}
      </div>

    </Sidebar>

    {/*
      右边这块平时是**空的**——后端把账号页面那个原生 webview 正好盖在这里。
      只有页面加载失败（后端会把 webview 藏起来）或者一个账号都没开时，
      下面画的东西才会露出来。不这么做的话，打不开的站就是一片白板，
      用户不知道是在转圈还是已经挂了。
    */}
    <div style={{ flexGrow: 1, minWidth: 0, display: "flex", flexDirection: "column", background: C.surface }}>
      {/*
        页签条。高度是 Layout 的 TABS_H，和 session.rs 的 TABS_H 对齐——
        后端把账号 webview 正好铺在它下面，所以**无论有没有页签都占着这块地方**，
        否则一开新页签整个页面会往下跳一格。
      */}
      <div
        style={{
          height: TABS_H,
          flexShrink: 0,
          boxSizing: "border-box",
          display: "flex",
          alignItems: "center",
          background: C.surface,
          borderBottom: `1px solid ${C.border}`,
          overflow: "hidden",
        }}
      >
        {/*
          页签可横向滚动，但浏览器控制组固定在右边：以前六个文字按钮跟页签挤在同一个
          滚动容器里，页签一多 / 窗口一窄，刷新和查找就被滚出屏幕了。
        */}
        <div
          style={{
            flexGrow: 1,
            minWidth: 0,
            alignSelf: "stretch",
            display: "flex",
            alignItems: "center",
            gap: 6,
            padding: "0 10px",
            overflowX: "auto",
            overflowY: "hidden",
          }}
        >
          {currentInfo && current ? (
            <>
              <span style={{ fontSize: 11, color: C.muted, flexShrink: 0, maxWidth: 130, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
                {current.related_app || current.username}
              </span>
              {currentInfo.tabs.map((tab, i) => {
                const on = i === currentInfo.active_tab && !tab.external;
                return (
                  <div
                    key={i}
                    style={{
                      display: "flex",
                      alignItems: "center",
                      flexShrink: 0,
                      maxWidth: 230,
                      borderRadius: "var(--sb-radius-sm)",
                      background: on ? C.bg : "transparent",
                      border: `1px solid ${on ? C.border : "transparent"}`,
                      paddingRight: 2,
                    }}
                  >
                    <button
                      type="button"
                      onClick={() => {
                        if (!on || tab.external) void run(async () => {
                          await api.selectSessionTab(current.id, i);
                          setSessions(await api.listActiveSessions());
                        });
                      }}
                      title={tab.external ? `${tab.url}（独立弹窗，点这里把窗口叫到前面）` : tab.url}
                      className="sb-pressable"
                      style={{
                        display: "flex",
                        alignItems: "center",
                        gap: 4,
                        minWidth: 0,
                        maxWidth: 170,
                        padding: "4px 4px 4px 9px",
                        border: 0,
                        borderRadius: "var(--sb-radius-sm)",
                        background: "transparent",
                        color: on ? C.text : C.sub,
                        fontSize: 12,
                        cursor: on ? "default" : "pointer",
                      }}
                    >
                      {tab.external && <IconExternal size={12} />}
                      <span style={{ overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
                        {tab.title.trim() || hostOf(tab.url) || (i === 0 ? "主页面" : tab.external ? "弹窗" : "新页签")}
                        {tab.failed && " · 打不开"}
                        {!tab.external && !tab.failed && !tab.loaded && " · 加载中"}
                      </span>
                    </button>
                    <IconButton
                      label="复制此页签链接"
                      title="复制此页签链接"
                      size="xs"
                      onClick={() => void run(async () => {
                        const latest = (await api.listActiveSessions()).find(s => s.account_id === current.id)?.tabs.find(t => t.label === tab.label);
                        if (!latest?.url) throw new Error("这个页签还没有地址");
                        if (!(await copyText(latest.url))) throw new Error("复制失败，请检查剪贴板权限");
                        setTip(`已复制地址：${latest.url}`);
                      })}
                    >
                      <IconCopy size={12} />
                    </IconButton>
                    {/* 首个页签不给关：关掉等于关账号，那件事归左边账号上的 × */}
                    {i > 0 && (
                      <IconButton
                        label="关掉这个页签"
                        title="关掉这个页签"
                        size="xs"
                        onClick={() => void run(async () => {
                          await api.closeSessionTab(current.id, i);
                          setSessions(await api.listActiveSessions());
                        })}
                      >
                        <IconClose size={12} />
                      </IconButton>
                    )}
                  </div>
                );
              })}
              {/* 别人发来的要登录才能看的链接：在这个账号下开，带着它的登录态 */}
              {newUrl === null ? (
                <IconButton
                  label="在这个账号里打开一个地址"
                  title={
                    currentState === "cut"
                      ? "页面已被区域拦截断开，先在告警框里重新检测网络"
                      : "在这个账号里打开一个地址（带着它的登录态，要登录才能看的链接也能直接看）"
                  }
                  size="sm"
                  disabled={currentState === "cut"}
                  onClick={() => setNewUrl("")}
                  style={{ border: 0, background: "transparent" }}
                >
                  <IconPlus size={15} />
                </IconButton>
              ) : (
                <input
                  autoFocus
                  value={newUrl}
                  aria-label="要在这个账号里打开的地址"
                  placeholder="粘贴地址，回车在这个账号里打开"
                  onChange={(e) => setNewUrl(e.target.value)}
                  onBlur={() => setNewUrl(null)}
                  onKeyDown={(e) => {
                    if (e.key === "Escape") setNewUrl(null);
                    if (e.key !== "Enter" || e.nativeEvent.isComposing || !newUrl.trim()) return;
                    const url = newUrl;
                    setNewUrl(null);
                    void run(async () => {
                      await api.openUrlInSession(current.id, url);
                      setSessions(await api.listActiveSessions());
                    });
                  }}
                  style={{ flexShrink: 0, width: 260, padding: "4px 8px", borderRadius: "var(--sb-radius-sm)", border: `1px solid ${C.brand}`, fontSize: 12, color: C.text, background: C.surface }}
                />
              )}
            </>
          ) : (
            <span style={{ fontSize: 12, color: C.muted }}>没有打开的页面</span>
          )}
        </div>

        {currentInfo && current && (() => {
          const url = currentInfo.tabs[currentInfo.active_tab]?.url ?? "";
          return (
            <div
              style={{
                flexShrink: 0,
                alignSelf: "stretch",
                display: "flex",
                alignItems: "center",
                gap: 2,
                padding: "0 10px",
                borderLeft: `1px solid ${C.border}`,
                background: C.surface,
              }}
            >
              <IconButton label="后退" title="后退" size="sm" onClick={() => run(() => api.tabHistory(current.id, -1))}>
                <IconChevronLeft size={15} />
              </IconButton>
              <IconButton label="前进" title="前进" size="sm" onClick={() => run(() => api.tabHistory(current.id, 1))}>
                <IconChevronRight size={15} />
              </IconButton>
              <IconButton label="刷新这个页签" title="刷新这个页签" size="sm" onClick={() => run(() => api.reloadSession(current.id))}>
                <IconReload size={15} />
              </IconButton>
              <IconButton label="在页面里查找" title={`在页面里查找（${MOD}F）`} size="sm" onClick={() => run(() => api.findInPage(current.id))}>
                <IconSearch size={15} />
              </IconButton>
              <IconButton
                label="复制当前地址"
                title={url || "当前页签还没有地址"}
                size="sm"
                onClick={() => run(async () => {
                  if (!url) throw new Error("这个页签还没有地址");
                  // 当前页面的地址不是账号库里的凭据，不过主密码那道闸
                  if (!(await copyText(url))) throw new Error("复制失败，请检查剪贴板权限");
                  setTip(`已复制地址：${url}`);
                })}
              >
                <IconCopy size={15} />
              </IconButton>
              <IconButton
                label="在系统浏览器里打开"
                title="在系统默认浏览器里打开这个地址（会先过一次网络区域检测）"
                size="sm"
                onClick={() => run(async () => {
                  if (!url) throw new Error("这个页签还没有地址");
                  await api.openInSystemBrowser(url);
                  setTip("已交给系统浏览器打开");
                })}
              >
                <IconExternal size={15} />
              </IconButton>
            </div>
          );
        })()}
      </div>

      <div style={{ flexGrow: 1, minHeight: 0, display: "flex", alignItems: "center", justifyContent: "center", padding: 32 }}>
      {currentState === "cut" ? (
        <div style={{ maxWidth: 420, display: "flex", flexDirection: "column", gap: 12, textAlign: "center" }}>
          <div style={{ fontSize: 17, color: C.text }}>已被网络区域检测断开</div>
          <div style={{ fontSize: 13, color: C.sub, lineHeight: 1.9 }}>
            检测到境外出口或分流代理 / VPN，页面已经断开，不会再发出任何请求。
            <br />
            登录态存好了，关掉代理 / VPN 后会自动接回来，不用重新登录。
          </div>
          <div style={{ fontSize: 11, color: C.muted, lineHeight: 1.7 }}>
            会按设置的间隔自动重测。不想要这个行为的话，去「设置 → 网络区域检测」关掉。
          </div>
        </div>
      ) : currentState === "failed" && current ? (
        <div style={{ maxWidth: 420, display: "flex", flexDirection: "column", gap: 14, textAlign: "center" }}>
          <div style={{ fontSize: 17, color: C.text }}>这个页面打不开</div>
          <div style={{ fontSize: 13, color: C.danger, lineHeight: 1.7 }}>{byId.get(current.id)?.fail_reason}</div>
          <div style={{ fontSize: 13, color: C.sub, lineHeight: 1.9, textAlign: "left" }}>
            常见原因：
            <br />· 站点限制了访问来源 IP（宝塔面板这类很常见）
            <br />· 地址写错了，或服务没在跑
            <br />· 端口被防火墙挡了
          </div>
          <div
            style={{
              fontSize: 11,
              color: C.muted,
              background: C.bg,
              borderRadius: "var(--sb-radius-md)",
              padding: "10px 12px",
              wordBreak: "break-all",
              fontFamily: "ui-monospace, monospace",
              textAlign: "left",
            }}
          >
            {activeTab?.url || current.login_url || "（没填登录直达 URL）"}
          </div>
          {activeTab?.scheme_implied && (
            <div style={{ fontSize: 12, color: C.sub, lineHeight: 1.7 }}>
              这个地址没写协议头，已按 {activeTab.url.startsWith("https:") ? "https" : "http"} 打开。
              {activeTab.url.startsWith("https:") ? "如果站点只支持 http，点下面的按钮换一下。" : "如果站点只支持 https，点下面的按钮换一下。"}
            </div>
          )}
          <div style={{ display: "flex", gap: 10, justifyContent: "center", flexWrap: "wrap" }}>
            <Button kind="primary" onClick={() => run(() => api.reloadSession(current.id))}>
              刷新重试
            </Button>
            {activeTab?.manual_url && (
              <Button onClick={() => run(() => api.retryActiveTab(current.id, altScheme))}>
                改用 {altScheme}:// 重试
              </Button>
            )}
            <Button
              onClick={() =>
                run(async () => {
                  const log = await api.logPath().catch(() => null);
                  const text = [
                    "Switchboard 页面打不开",
                    `平台：${current.platform}`,
                    `地址：${activeTab?.url || current.login_url || "（空）"}`,
                    `原因：${byId.get(current.id)?.fail_reason ?? "未知"}`,
                    `手动输入：${activeTab?.manual_url ? "是" : "否"}${activeTab?.scheme_implied ? "（协议是按主机猜的）" : ""}`,
                    `日志：${log || "（未启用）"}`,
                  ].join("\n");
                  if (!(await copyText(text))) throw new Error("复制失败，请检查剪贴板权限");
                  setTip("诊断信息已复制，可以直接贴到 issue 里");
                })
              }
            >
              复制诊断信息
            </Button>
            <Button
              onClick={() =>
                run(async () => {
                  await api.closeSession(current.id);
                  setSessions(await api.listActiveSessions());
                })
              }
            >
              关闭
            </Button>
          </div>
          <div style={{ fontSize: 11, color: C.muted, lineHeight: 1.7 }}>
            换了 IP 之后记得点「刷新重试」——页面不会自己重连。
          </div>
        </div>
        ) : sessions.length === 0 ? (
          <div style={{ fontSize: 13, color: C.muted }}>从左边选一个账号打开</div>
        ) : null}
      </div>
    </div>

    {closing && (
      <Dialog label="关闭账号页面" width={400} onClose={() => setClosing(null)}>
        <div style={{ padding: 24, display: "flex", flexDirection: "column", gap: 14 }}>
          <div style={{ fontSize: 17, color: C.text, fontWeight: 500 }}>
            关闭「{closing.related_app || closing.username}」的页面？
          </div>
          <div style={{ fontSize: 13, color: C.sub, lineHeight: 1.7 }}>
            右侧页面会关闭，页面里没提交的表单内容会丢。登录状态已经保存，下次打开不用重新登录。
          </div>
          <div style={{ display: "flex", gap: 10, justifyContent: "flex-end" }}>
            <Button size="md" onClick={() => setClosing(null)}>取消</Button>
            <Button
              size="md"
              kind="primary"
              onClick={() => {
                const a = closing;
                setClosing(null);
                void run(async () => {
                  await api.closeSession(a.id);
                  setSessions(await api.listActiveSessions());
                }, "已关闭页面，登录状态已保存");
              }}
            >
              关闭页面
            </Button>
          </div>
        </div>
      </Dialog>
    )}

    {clearing && (
      <Dialog label="清除登录状态" width={420} onClose={() => setClearing(null)}>
        <div style={{ padding: 24, display: "flex", flexDirection: "column", gap: 14 }}>
          <div style={{ fontSize: 17, color: C.danger, fontWeight: 500 }}>
            清除「{clearing.related_app || clearing.username}」的登录状态？
          </div>
          <div style={{ fontSize: 13, color: C.sub, lineHeight: 1.7 }}>
            会关闭右侧页面，并删掉这个账号保存的 cookie 快照和本机登录数据。下次打开需要重新登录，且无法撤销。
          </div>
          <div style={{ display: "flex", gap: 10, justifyContent: "flex-end" }}>
            <Button size="md" onClick={() => setClearing(null)}>取消</Button>
            <Button
              size="md"
              kind="danger"
              onClick={() => {
                const a = clearing;
                setClearing(null);
                void run(async () => {
                  await api.clearLogin(a.id);
                  setSessions(await api.listActiveSessions());
                }, "已清除这个账号保存的登录状态，页面已关闭；再打开需要重新登录");
              }}
            >
              清除登录状态
            </Button>
          </div>
        </div>
      </Dialog>
    )}
    </>
  );
}

/** "上次保活"用的相对时间 */
function ago(secs: number | null): string {
  if (secs == null) return "刚刚";
  if (secs < 60) return `${secs} 秒前`;
  const m = Math.floor(secs / 60);
  if (m < 60) return `${m} 分钟前`;
  return `${Math.floor(m / 60)} 小时 ${m % 60} 分钟前`;
}

/** "还有多久刷新" */
function untilRefresh(secs: number | null): string {
  if (secs == null) return "马上刷新";
  if (secs <= 5) return "马上刷新";
  if (secs < 60) return `约 ${secs} 秒后刷新`;
  return `约 ${Math.ceil(secs / 60)} 分钟后刷新`;
}

/** 页签上没有标题时退回显示主机名，整条 URL 太长塞不下 */
function hostOf(url: string): string {
  try {
    return new URL(url).host;
  } catch {
    return "";
  }
}
