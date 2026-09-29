import { useEffect, useMemo, useState } from "react";
import { save } from "@tauri-apps/plugin-dialog";
import * as api from "../api";
import type { Account, SessionInfo } from "../api";
import { Button, C, FILE_MANAGER, MOD, copySecret, copyText } from "../ui";
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
  /** 最近一次下载成功的文件，点提示条在访达里选中它 */
  const [dl, setDl] = useState<{ name: string; path: string } | null>(null);
  const [query, setQuery] = useState("");
  const [onlyOnline, setOnlyOnline] = useState(false);
  const [showAll, setShowAll] = useState(false);
  /** 当前账号的「更多操作」展开没有。切账号就收起 */
  const [more, setMore] = useState(false);
  /** 页签条上的地址输入框。null = 收起 */
  const [newUrl, setNewUrl] = useState<string | null>(null);

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

  // 页签里的下载。落盘位置是后端定的（设置里选的下载文件夹），这里报一声在哪、给个一点就能找到的入口，
  // 否则用户只看到"下完了"却不知道文件去了哪
  useEffect(() => {
    const un = api.onDownloadFinished(async (d) => {
      if (!d.success) {
        setTip("");
        setErr(`下载失败：${d.name || "文件"}`);
        return;
      }
      if (!d.ask) return setDl(d);
      // 「每次都问」：文件已经落在下载文件夹里了，再问存到哪、挪过去（为什么不先问见 lib.rs 的 move_download）。
      // 点取消就留在原地，照常提示
      const to = await save({ title: `保存「${d.name}」`, defaultPath: d.path }).catch(() => null);
      if (!to) return setDl(d);
      try {
        const path = await api.moveDownload(d.path, to);
        setDl({ name: path.split(/[\\/]/).pop() || d.name, path });
      } catch (e) {
        setDl(d);
        setTip("");
        setErr(String(e));
      }
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

  async function open(a: Account) {
    setBusy(a.id);
    await run(async () => {
      await onOpen(a);
      setActive(await api.activeSession());
      setSessions(await api.listActiveSessions());
    });
    setBusy("");
  }

  const toastStyle = { padding: "8px 10px", marginBottom: 8, borderRadius: 7, fontSize: 11, lineHeight: 1.6, cursor: "pointer" } as const;

  const action = (label: string, onClick: () => void, title?: string) => (
    <div
      onClick={onClick}
      title={title}
      style={{
        padding: "6px 8px",
        borderRadius: 6,
        border: `1px solid ${C.border}`,
        background: C.surface,
        fontSize: 11,
        color: C.sub,
        cursor: "pointer",
        textAlign: "center",
        flexGrow: 1,
        minWidth: 0,
        overflow: "hidden",
        textOverflow: "ellipsis",
        whiteSpace: "nowrap",
      }}
    >
      {label}
    </div>
  );

  return (
    <>
    <Sidebar
      footer={
        <>
          {/* 提示条钉在底部导航上方，不跟账号列表一起滚：账号一多它就被挤出可视区，
              右上角按钮点了、报错了，用户看到的却是"没反应" */}
          {dl && (
            <div
              onClick={() => {
                setDl(null);
                api.revealDownload(dl.path).catch((e) => setErr(String(e)));
              }}
              title={dl.path}
              style={{ ...toastStyle, background: C.brandSoft, color: C.brand }}
            >
              已下载「{dl.name}」· 点这里在{FILE_MANAGER}中显示
            </div>
          )}
          {(tip || err) && (
            <div
              onClick={() => {
                setTip("");
                setErr("");
              }}
              style={{
                ...toastStyle,
                background: err ? "rgba(227,77,89,0.08)" : C.brandSoft,
                color: err ? C.danger : C.brand,
              }}
            >
              {err || tip}
            </div>
          )}
          <SideNav current="sessions" sessionCount={sessionCount} onGo={onGo} onLock={onLock} />
        </>
      }
    >
      <div style={{ display: "flex", flexDirection: "column", gap: 8 }}>
        <div style={{ fontSize: 11, color: C.muted, padding: "0 8px" }}>账号 · {online.size} 个已打开 / {accounts.length} 个</div>
        <Button onClick={onAdd} style={{ padding: "7px 10px", fontSize: 11 }}>+ 新增账号</Button>
        <input
          type="search"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          placeholder="搜索账号、归属或平台"
          aria-label="搜索会话账号"
          style={{ width: "100%", boxSizing: "border-box", height: 32, border: `1px solid ${C.border}`, borderRadius: 7, background: C.surface, padding: "0 9px", fontSize: 11, color: C.text, outlineColor: C.brand }}
        />
        <div style={{ display: "flex", gap: 6 }}>
          {([false, true] as const).map((onlineOnly) => (
            <button
              key={String(onlineOnly)}
              type="button"
              onClick={() => setOnlyOnline(onlineOnly)}
              style={{ flex: 1, border: `1px solid ${onlyOnline === onlineOnly ? C.brand : C.border}`, background: onlyOnline === onlineOnly ? C.brandSoft : C.surface, color: onlyOnline === onlineOnly ? C.brand : C.sub, borderRadius: 6, padding: "5px 0", fontSize: 11, cursor: "pointer" }}
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
            <div key={a.id}>
              <div
                onClick={() => open(a)}
                title={`${a.owner_type} · ${a.platform} · ${a.related_app || a.username}`}
                style={{
                  display: "flex",
                  alignItems: "center",
                  gap: 8,
                  padding: "6px 8px",
                  borderRadius: 7,
                  background: cur ? C.surface : "transparent",
                  border: `1px solid ${cur ? C.border : "transparent"}`,
                  cursor: "pointer",
                }}
              >
                <span style={{ width: 6, height: 6, borderRadius: 999, flexShrink: 0, background: on ? C.success : C.borderStrong }} />
                <div style={{ display: "flex", flexDirection: "column", gap: 1, minWidth: 0, flexGrow: 1 }}>
                  <span style={{ fontSize: 12, color: cur ? C.text : C.sub, whiteSpace: "nowrap", overflow: "hidden", textOverflow: "ellipsis" }}>
                    {a.related_app || a.username}
                  </span>
                  <span style={{ fontSize: 11, color: C.muted, whiteSpace: "nowrap", overflow: "hidden", textOverflow: "ellipsis" }}>
                    {/* 归属放前面：同平台同名的两个账号，能分清的就是它 */}
                    <span style={{ color: C.sub }}>{a.owner_type}</span> · {a.platform}
                    {busy === a.id && " · 打开中…"}
                    {busy !== a.id && stateOf(a.id) === "loading" && " · 加载中…"}
                    {stateOf(a.id) === "failed" && <span style={{ color: C.danger }}> · 打不开</span>}
                    {stateOf(a.id) === "cut" && <span style={{ color: C.danger }}> · 已断开</span>}
                  </span>
                </div>
                {on && (
                  <span
                    onClick={(e) => {
                      e.stopPropagation();
                      run(async () => {
                        await api.closeSession(a.id);
                        setSessions(await api.listActiveSessions());
                      });
                    }}
                    title="关闭这个账号"
                    style={{ fontSize: 13, color: C.muted, flexShrink: 0, padding: "0 2px", lineHeight: 1 }}
                  >
                    ×
                  </span>
                )}
              </div>

              {/*
                当前账号的操作挂在它自己下面。只露最常用的填充，其余收进「更多」：
                全摊开是十来个按钮，会把别的在线账号挤出屏幕，多账号切换反倒找不到人。
                「更多」就地展开而不是弹出菜单——右边整块是原生 webview，HTML 弹层会被它盖住
              */}
              {cur && on && current && (
                <div style={{ padding: "6px 4px 10px", display: "flex", flexDirection: "column", gap: 6 }}>
                  {/* 页面没加载出来时填充无从谈起，与其摆一排点了就报错的按钮，不如直说 */}
                  {currentState === "ok" ? (
                    <div style={{ display: "flex", gap: 6 }}>
                      {action("填账号", () => fill(a.id, "username", "账号"), fillTip)}
                      {action("填密码", () => fill(a.id, "password", "密码"), fillTip)}
                      {current.totp_secret && action("填验证码", () => fill(a.id, "totp", "验证码"), fillTip)}
                    </div>
                  ) : (
                    <div style={{ fontSize: 11, color: C.muted, lineHeight: 1.6 }}>
                      {currentState === "failed" ? "页面没打开，先刷新" : "页面加载中…"}
                    </div>
                  )}
                  <button
                    type="button"
                    onClick={() => setMore(!more)}
                    style={{ border: 0, background: "transparent", color: C.sub, fontSize: 11, padding: "2px 0", cursor: "pointer" }}
                  >
                    {more ? "收起 ▴" : "更多操作 ▾"}
                  </button>
                  {more && (
                    <div style={{ display: "grid", gridTemplateColumns: "repeat(2, minmax(0, 1fr))", gap: 6 }}>
                      {currentState === "ok" && current.extra_fields.map((field, index) => field.value ? (
                        <div key={index} style={{ minWidth: 0, display: "flex" }}>
                          {action(`填${field.label}`, () => fill(a.id, `extra:${index}`, field.label), fillTip)}
                        </div>
                      ) : null)}
                      {/* 刷新任何时候都得在：页面打不开的时候恰恰是最需要它的时候 */}
                      {action("刷新页面", () => run(() => api.reloadSession(a.id)), "重新加载这个页面")}
                      {action("编辑账号", () => onEdit(a), "修改当前账号及附加字段")}
                      {/* 「设为直达页」会去问 WKWebView 要 URL，页面没加载出来时它是 nil，见 session.rs 的 url_is_safe */}
                      {currentState === "ok" && action(
                        "设为直达页",
                        () =>
                          run(async () => {
                            await api.pinCurrentUrl(a.id);
                            onAccountsChanged();
                          }, "已把当前页面设为该账号的登录直达 URL"),
                        "把右边现在停的这个地址存成该账号的登录直达 URL，下次直接开这里",
                      )}
                      {currentState === "ok" && action(
                        "指认输入框",
                        () =>
                          run(
                            () => api.setPickMode(a.id, true),
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
                        () =>
                          run(async () => {
                            await api.clearLogin(a.id);
                            setSessions(await api.listActiveSessions());
                          }, "已清除这个账号保存的登录状态，页面已关闭；再打开需要重新登录"),
                        "关掉页面，并删掉这个账号保存的 cookie 和本机登录数据，下次打开需要重新登录",
                      )}
                    </div>
                  )}
                </div>
              )}
            </div>
          );
        })}
        {hiddenCount > 0 && (
          <button
            type="button"
            onClick={() => setShowAll(true)}
            style={{ border: 0, background: "transparent", color: C.brand, cursor: "pointer", textAlign: "left", padding: "8px 10px", fontSize: 11 }}
          >
            显示其余 {hiddenCount} 个账号
          </button>
        )}
        {showAll && !keyword && !onlyOnline && matching.length > 6 && (
          <button
            type="button"
            onClick={() => setShowAll(false)}
            style={{ border: 0, background: "transparent", color: C.brand, cursor: "pointer", textAlign: "left", padding: "8px 10px", fontSize: 11 }}
          >
            收起账号列表
          </button>
        )}
        {matching.length === 0 && (
          <div style={{ padding: "8px 10px", fontSize: 11, color: C.muted }}>
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
    <div style={{ flexGrow: 1, minWidth: 0, display: "flex", flexDirection: "column" }}>
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
          gap: 6,
          padding: "0 10px",
          background: C.bg,
          borderBottom: `1px solid ${C.border}`,
          overflowX: "auto",
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
                  onClick={() => {
                    if (!on || tab.external) void run(async () => {
                      await api.selectSessionTab(current.id, i);
                      setSessions(await api.listActiveSessions());
                    });
                  }}
                  title={tab.external ? `${tab.url}（独立弹窗，点这里把窗口叫到前面）` : tab.url}
                  style={{
                    display: "flex",
                    alignItems: "center",
                    gap: 6,
                    flexShrink: 0,
                    maxWidth: 200,
                    padding: "4px 9px",
                    borderRadius: 6,
                    // 选中态跟侧栏导航一个做法：白底 + 描边 + 轻阴影，不"忽然变白"
                    background: on ? C.surface : "transparent",
                    border: `1px solid ${on ? C.border : "transparent"}`,
                    boxShadow: on ? "0 1px 2px rgba(0,0,0,0.05)" : "none",
                    color: on ? C.text : C.sub,
                    fontSize: 11,
                    cursor: on ? "default" : "pointer",
                  }}
                >
                  <span style={{ overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
                    {/* 弹窗是独立窗口，标个 ↗ 提示"点它是把那个窗口叫到前面"，不是在这儿切 */}
                    {tab.external && "↗ "}
                    {tab.title.trim() || hostOf(tab.url) || (i === 0 ? "主页面" : tab.external ? "弹窗" : "新页签")}
                    {tab.failed && " · 打不开"}
                    {!tab.external && !tab.failed && !tab.loaded && " · 加载中"}
                  </span>
                  {/* 首个页签不给关：关掉等于关账号，那件事归左边账号上的 × */}
                  {i > 0 && (
                    <span
                      onClick={(e) => {
                        e.stopPropagation();
                        run(async () => {
                          await api.closeSessionTab(current.id, i);
                          setSessions(await api.listActiveSessions());
                        });
                      }}
                      title="关掉这个页签"
                      style={{ color: C.muted, flexShrink: 0, lineHeight: 1 }}
                    >
                      ×
                    </span>
                  )}
                </div>
              );
            })}
            {/* 别人发来的要登录才能看的链接：在这个账号下开，带着它的登录态 */}
            {newUrl === null ? (
              <button
                type="button"
                onClick={() => setNewUrl("")}
                title="在这个账号里打开一个地址（带着它的登录态，要登录才能看的链接也能直接看）"
                style={{ flexShrink: 0, padding: "2px 8px", border: 0, borderRadius: 6, background: "transparent", color: C.sub, fontSize: 14, lineHeight: 1, cursor: "pointer" }}
              >
                +
              </button>
            ) : (
              <input
                autoFocus
                value={newUrl}
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
                style={{ flexShrink: 0, width: 280, padding: "3px 8px", borderRadius: 6, border: `1px solid ${C.brand}`, outline: "none", fontSize: 11, color: C.text, background: C.surface }}
              />
            )}
            {/* 浏览器该有的那几下。页签条右端，只占一行 */}
            {(() => {
              const url = currentInfo.tabs[currentInfo.active_tab]?.url ?? "";
              const btn = (label: string, title: string, onClick: () => void) => (
                <button
                  key={label}
                  type="button"
                  onClick={onClick}
                  title={title}
                  style={{
                    flexShrink: 0,
                    padding: "3px 8px",
                    borderRadius: 6,
                    border: `1px solid ${C.border}`,
                    background: C.surface,
                    color: C.sub,
                    fontSize: 11,
                    lineHeight: 1.4,
                    cursor: "pointer",
                  }}
                >
                  {label}
                </button>
              );
              return (
                <div style={{ marginLeft: "auto", display: "flex", alignItems: "center", gap: 4, flexShrink: 0, paddingLeft: 8 }}>
                  {btn("‹", "后退", () => run(() => api.tabHistory(current.id, -1)))}
                  {btn("›", "前进", () => run(() => api.tabHistory(current.id, 1)))}
                  {btn("⟳", "刷新这个页签", () => run(() => api.reloadSession(current.id)))}
                  {btn("查找", `在页面里查找（${MOD}F）`, () => run(() => api.findInPage(current.id)))}
                  {btn("复制地址", url || "当前页签还没有地址", () =>
                    run(async () => {
                      if (!url) throw new Error("这个页签还没有地址");
                      // 当前页面的地址不是账号库里的凭据，不过主密码那道闸
                      if (!(await copyText(url))) throw new Error("复制失败，请检查剪贴板权限");
                      setTip(`已复制地址：${url}`);
                    }),
                  )}
                  {btn("系统浏览器", "在系统默认浏览器里打开这个地址（会先过一次网络区域检测）", () =>
                    run(async () => {
                      if (!url) throw new Error("这个页签还没有地址");
                      await api.openInSystemBrowser(url);
                      setTip("已交给系统浏览器打开");
                    }),
                  )}
                </div>
              );
            })()}
          </>
        ) : (
          <span style={{ fontSize: 11, color: C.muted }}>没有打开的页面</span>
        )}
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
              borderRadius: 8,
              padding: "10px 12px",
              wordBreak: "break-all",
              fontFamily: "ui-monospace, monospace",
              textAlign: "left",
            }}
          >
            {(currentInfo && currentInfo.active_tab > 0 ? currentInfo.tabs[currentInfo.active_tab]?.url : current.login_url) || "（没填登录直达 URL）"}
          </div>
          <div style={{ display: "flex", gap: 10, justifyContent: "center" }}>
            <Button kind="primary" onClick={() => run(() => api.reloadSession(current.id))}>
              刷新重试
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
    </>
  );
}

/** 页签上没有标题时退回显示主机名，整条 URL 太长塞不下 */
function hostOf(url: string): string {
  try {
    return new URL(url).host;
  } catch {
    return "";
  }
}
