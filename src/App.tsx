import { useCallback, useEffect, useRef, useState } from "react";
import * as api from "./api";
import type { Account, SessionInfo } from "./api";
import { Register, Unlock } from "./screens/Auth";
import Main from "./screens/Main";
import AccountForm from "./screens/AccountForm";
import DeleteConfirm from "./screens/DeleteConfirm";
import Settings from "./screens/Settings";
import PlatformConfigScreen from "./screens/PlatformConfig";
import AdaptHelp from "./screens/AdaptHelp";
import SessionsPage from "./screens/SessionsPage";
import { CopyGate, QuickSwitch, TotpCode } from "./screens/Overlays";
import { RegionAlarm, RegionBlocked } from "./screens/RegionGuard";
import { AppShell, type Page, type SubPageShell } from "./screens/Layout";
import { C, ErrorLine, MOD, copySecret } from "./ui";

type Screen =
  | { name: "loading" }
  | { name: "register" }
  | { name: "unlock"; reason?: string }
  | { name: "main" }
  // back：存完 / 取消回哪。会话页里点的编辑、新增，存完要回会话页，不能把人扔回账号列表
  | { name: "form"; account: Account; isNew: boolean; copiedFrom?: string; back: "main" | "sessions" }
  | { name: "sub"; page: "settings" | "platforms" }
  | { name: "sessions" };

type Overlay =
  | { name: "none" }
  | { name: "delete"; account: Account }
  | { name: "totp"; account: Account }
  | { name: "quickswitch" }
  // 复制明文前的主密码闸门。text 只在这里停留到验证通过为止，验完即写剪贴板
  | { name: "copy"; label: string; text: api.CopySource }
  | { name: "region"; account: Account; verdict: api.Verdict; mode: api.RegionMode }
  // 哨兵在运行途中探到网络变了。跟上面那个不一样：那个是"点账号时被挡下"，
  // 这个是"用着用着网络自己变了"，此时页面可能已经被后端断掉了
  | { name: "alarm"; verdict: api.Verdict; cut: boolean };

/** 验过一次主密码后的免验时长。连着复制账号和密码不该被打断两回 */
const COPY_GRACE_MS = 5 * 60 * 1000;

/** 自动锁定的空闲检查频率。它就是"超时之后界面还能被人看见"的最长时间，所以别太长 */
const AUTO_LOCK_CHECK_MS = 5 * 1000;
const AUTO_LOCK_REASON = "长时间未操作，已自动锁定";

/** 取消和失败必须分开：取消不是错误，卡片上不该显示"复制失败" */
export type CopyResult = "done" | "error" | "cancel";

export default function App() {
  const [screen, setScreen] = useState<Screen>({ name: "loading" });
  const [overlay, setOverlay] = useState<Overlay>({ name: "none" });
  const [accounts, setAccounts] = useState<Account[]>([]);
  const [sessions, setSessions] = useState<SessionInfo[]>([]);
  const [err, setErr] = useState("");
  const [help, setHelp] = useState(false);
  // 导入配置后用它强制重挂平台配置表，省得另写一套刷新回调
  const [configVersion, setConfigVersion] = useState(0);
  // 从「平台适配配置」点「指认输入框」过来的账号。页面要先加载出来才能进指认模式，
  // 所以这里只记下目标，由会话页盯着加载状态，到点了自己开
  const [pickTarget, setPickTarget] = useState<string | null>(null);
  // 闸门：免验截止时间 + 正在等结果的那个复制请求
  const copyFreeUntil = useRef(0);
  const copyWaiting = useRef<((r: CopyResult) => void) | null>(null);
  // 闸门是顶掉当前浮层弹出来的（浮层只有一个）。验完要把原来那个放回去，
  // 否则在「验证码」框里点一下数字，倒计时框就没了，用户也不知道复制成没成
  const copyReturnTo = useRef<Overlay>({ name: "none" });
  const [region, setRegion] = useState<api.RegionState | null>(null);
  const [checkingRegion, setCheckingRegion] = useState(false);
  const [regionNotice, setRegionNotice] = useState("");
  // 自动锁定：0 = 关闭（默认）。"空闲了多久"由后端统一计时，见下面的 useEffect
  const [autoLockSecs, setAutoLockSecs] = useState(0);
  // 一键自动演示：顶栏上的解说字幕；running=false 时只是收尾提示（录屏存哪了），不给"停止"
  const [tour, setTour] = useState<{ text: string; running: boolean } | null>(null);
  const tourStop = useRef(false);

  const unlocked = ["main", "form", "sub", "sessions"].includes(screen.name);

  useEffect(() => {
    api.isInitialized().then((ok) => setScreen(ok ? { name: "unlock" } : { name: "register" }));
  }, []);

  // 会话是内存态，解锁期间定时对一次账
  useEffect(() => {
    if (!unlocked) return;
    const tick = () => api.listActiveSessions().then(setSessions).catch(() => {});
    tick();
    const t = window.setInterval(tick, 3000);
    return () => window.clearInterval(t);
  }, [unlocked]);

  // 后端哨兵按设置的间隔探测网络，结果推过来。
  // block 模式下后端**已经自己断完了**（cut=true），这里只负责告诉用户发生了什么；
  // warn 模式后端不动手，由这个框问用户。弹不弹由后端的 ask 决定，前端不重复判。
  useEffect(() => {
    const un = api.onRegionChanged((e) => {
      setRegion((r) => (r ? { ...r, verdict: e.verdict, mode: e.mode, cut: e.cut } : r));
      // 网络自己好了（后端已经把页面接回来了），框自动撤掉
      if (!api.regionNeedsHandling(e.verdict)) {
        setOverlay((o) => (o.name === "alarm" ? { name: "none" } : o));
        return;
      }
      if (e.ask) setOverlay({ name: "alarm", verdict: e.verdict, cut: e.cut });
    });
    return () => {
      void un.then((f) => f());
    };
  }, []);

  const enterMain = useCallback(async () => {
    setErr("");
    setAccounts(await api.listAccounts());
    setScreen({ name: "main" });
    // 徽标用；用缓存不强制重测，不阻塞进入主界面
    api.checkRegion(false)
      .then(async (state) => state.mode !== "off" && api.regionNeedsHandling(state.verdict) ? api.patrolRegion() : state)
      .then(setRegion)
      .catch(() => {});
    api.getAutoLockSecs().then(setAutoLockSecs).catch(() => {});
  }, []);

  // 长时间不操作自动锁回登录页。"空闲了多久"问后端要（`api.idleSecs`）：
  // 账号页面（会话页右侧）是独立的原生 webview，里面的操作不会冒泡到这儿，
  // 得靠 adapter.rs 注入的脚本把心跳报给后端，两边共用同一份时钟（睡眠也算在内）。
  // 解锁时后端已经重新起算，这里不用先 touch 一次。
  useEffect(() => {
    if (!unlocked || autoLockSecs <= 0) return;
    let locking = false;
    const lockNow = () => {
      if (locking) return;
      locking = true;
      doLock(AUTO_LOCK_REASON).catch(() => { locking = false; });
    };
    const check = () => {
      api.idleSecs().then((idle) => { if (idle >= autoLockSecs) lockNow(); }).catch(() => {});
    };
    // 主窗口自己的操作节流着报给后端。后端说已经超时（人离开太久、回来随手一点）就当场锁——
    // 那一下点击不续命，否则赶在下一轮检查前点一下就能绕过自动锁定
    let lastSent = 0;
    const touch = () => {
      const now = Date.now();
      if (now - lastSent < 5000) return; // 节流，别把 IPC 打爆
      lastSent = now;
      api.touchActivity().then((expired) => { if (expired) lockNow(); }).catch(() => {});
    };
    // 窗口藏起来时 macOS 会给计时器降频（App Nap），回到前台先查一次，别等下一轮
    const onVisible = () => { if (document.visibilityState === "visible") check(); };
    const events = ["mousedown", "keydown", "wheel", "touchstart"] as const;
    events.forEach((e) => window.addEventListener(e, touch));
    window.addEventListener("focus", check);
    document.addEventListener("visibilitychange", onVisible);
    const t = window.setInterval(check, AUTO_LOCK_CHECK_MS);
    return () => {
      events.forEach((e) => window.removeEventListener(e, touch));
      window.removeEventListener("focus", check);
      document.removeEventListener("visibilitychange", onVisible);
      window.clearInterval(t);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [unlocked, autoLockSecs]);

  // 账号页面是原生 webview，永远盖在主窗口网页之上，HTML 弹层压不住它：会话页上弹出来的
  // ⌘K、验证码、复制闸门、区域告警都会被挡住大半——warn 模式的告警框还不给 Esc 关、
  // 背景遮罩又盖住了侧栏，看起来就是整个 App 卡死了。弹层开着就先把账号页面藏起来
  useEffect(() => {
    if (screen.name !== "sessions") return;
    void api.setSessionsVisible(overlay.name === "none").catch(() => {});
  }, [screen.name, overlay.name]);

  const checkNetwork = useCallback(async () => {
    setCheckingRegion(true);
    setRegionNotice("");
    try {
      const s = await api.patrolRegion();
      setRegion(s);
      api.listActiveSessions().then(setSessions).catch(() => {});
      const v = s.verdict;
      const detail = v.ip ? `（${v.ip}）` : "";
      let notice: string;
      if (v.origin.kind === "mainland" && v.tunnel) {
        notice = `检测到分流代理 / VPN：国内探针出口为中国大陆${detail}，但外网探针可达。目标站点可能走境外节点。`;
      } else if (v.origin.kind === "outside") {
        notice = `检测到境外出口：${api.regionLabel(v)}${detail}。`;
      } else if (v.origin.kind === "unknown") {
        notice = `网络检测未得到明确结果：${v.origin.reason}。请检查网络或探测端点后重试。`;
      } else {
        notice = `当前出口：中国大陆${detail}。`;
      }
      if (api.regionNeedsHandling(v)) notice += s.mode === "off" ? "检测已关闭，本次仅显示结果。" : "已按当前检测模式处理。";
      setRegionNotice(notice);
    } catch (e) {
      setRegionNotice(`网络检测失败：${String(e)}`);
    } finally {
      setCheckingRegion(false);
    }
  }, []);

  const run = useCallback(async (f: () => Promise<unknown>) => {
    try {
      setErr("");
      await f();
    } catch (e) {
      setErr(String(e));
    }
  }, []);

  const goto = useCallback(
    (p: Page) => {
      if (p === "main") void enterMain();
      else if (p === "sessions") setScreen({ name: "sessions" });
      else setScreen({ name: "sub", page: p });
    },
    [enterMain],
  );

  /** force=true 表示用户在区域提示框里点了"继续"，跳过检查 */
  const launch = useCallback(
    (a: Account, force = false) =>
      run(async () => {
        const out = await api.switchOrOpenAccount(a.id, force);
        if (!out.opened && out.blocked) {
          // 境外出口或分流隧道，交给弹层让用户决定
          setOverlay({ name: "region", account: a, verdict: out.blocked, mode: out.mode });
          return;
        }
        setSessions(await api.listActiveSessions());
        setOverlay({ name: "none" });
        setScreen({ name: "sessions" });
      }),
    [run],
  );

  /** 平台适配配置里点「指认输入框」：优先用已经开着的那个账号，没有就现开一个 */
  const pickFor = useCallback(
    (platform: string) =>
      run(async () => {
        const cands = api.accountsForConfig(accounts, platform);
        const a = cands.find((x) => sessions.some((s) => s.account_id === x.id)) ?? cands[0];
        if (!a) throw new Error(`账号库里还没有「${platform}」的账号，先加一个再来指认`);
        setPickTarget(a.id);
        await launch(a); // 自己会切到会话页；被区域检测拦下时会弹框，那时指认自然也不该开始
      }),
    [accounts, sessions, launch, run],
  );

  /**
   * 所有"把明文放进剪贴板"的入口都走这里：宽限期内直接复制，否则先要主密码。
   * 返回三态而不是布尔——取消和复制失败对用户是两件事。
   */
  const requestCopy = useCallback((label: string, text: api.CopySource): Promise<CopyResult> => {
    if (typeof text === "string" && !text.trim()) return Promise.resolve<CopyResult>("error");
    if (Date.now() < copyFreeUntil.current) return copyFrom(text);
    return new Promise<CopyResult>((resolve) => {
      // 上一个还挂着的请求被顶掉了，得给它个结果，不然它的 await 永远回不来
      copyWaiting.current?.("cancel");
      copyWaiting.current = resolve;
      setOverlay((prev) => {
        copyReturnTo.current = prev.name === "copy" ? { name: "none" } : prev;
        return { name: "copy", label, text };
      });
    });
  }, []);

  /** 取到内容（可能要去后端拿明文）再复制，取不到或是空的都算失败 */
  const copyFrom = (text: api.CopySource): Promise<CopyResult> =>
    (typeof text === "string" ? Promise.resolve(text) : text())
      .then((t) => (t.trim() ? copySecret(t) : false))
      .then((ok) => (ok ? "done" : "error"), () => "error");

  const settleCopy = useCallback((r: CopyResult) => {
    copyWaiting.current?.(r);
    copyWaiting.current = null;
    setOverlay(copyReturnTo.current);
    copyReturnTo.current = { name: "none" };
  }, []);

  /** 列表里的账号打过码（见 lib.rs 的 masked），编辑 / 复制新建前先取完整的一条 */
  const openForm = (a: Account, back: "main" | "sessions", duplicate = false) =>
    run(async () => {
      const full = await api.getAccount(a.id);
      setScreen(
        duplicate
          ? { name: "form", account: { ...full, id: crypto.randomUUID() }, isNew: true, copiedFrom: full.related_app || full.username, back }
          : { name: "form", account: full, isNew: false, back },
      );
    });

  /** 账号改过之后（编辑、设为直达页）重新拉一次列表，别等切页才看到新值 */
  const refreshAccounts = useCallback(() => api.listAccounts().then(setAccounts).catch((e) => setErr(String(e))), []);

  /** 离开表单：回会话页只要刷新列表，回账号列表走 enterMain（那边还要对一次网络状态） */
  const leaveForm = async (back: "main" | "sessions") => {
    if (back === "main") return enterMain();
    await refreshAccounts();
    setScreen({ name: "sessions" });
  };

  /** 演示模式：后端换好假账号库并已解锁，直接进主界面 */
  const enterDemo = () => run(async () => {
    await api.enterDemo();
    await enterMain();
  });

  /**
   * 一键自动演示：从全新的假账号库开始，按脚本把主要功能走一遍，每步停几秒给观众看。
   * 页面里的"点登录"由模拟页自己做（set_demo_tour），这里只管 App 这一侧的操作。
   * 录屏按总时长定时录，录满自动收尾，所以脚本里每步的秒数就是视频节奏。
   */
  const runTour = (record: boolean) => run(async () => {
    await api.enterDemo();
    await enterMain();
    const list = await api.listAccounts();
    const a1 = list.find((a) => a.platform === "阿里云");
    const a2 = list.find((a) => a.platform === "腾讯云");
    if (!a1 || !a2) throw new Error("演示数据不全");
    const steps: [string, number, () => unknown][] = [
      ["账号库：各平台账号集中管理，按归属和平台筛选", 4, () => {}],
      ["一键登录：自动打开登录页、填好账号密码", 7, () => launch(a1)],
      ["多个账号同时在线，登录态互相隔离", 7, () => launch(a2)],
      ["左侧点一下就切换，不用重新登录", 3, () => launch(a1)],
      ["内置 TOTP 动态验证码，不用再掏手机", 4, () => setOverlay({ name: "totp", account: a1 })],
      [`${MOD}K 搜索账号，秒切换`, 4, () => setOverlay({ name: "quickswitch" })],
      ["演示结束", 2, () => setOverlay({ name: "none" })],
    ];
    const total = steps.reduce((sum, [, secs]) => sum + secs, 0);
    // 录不了（没权限）就别演了：用户要的是视频，白演一遍没用
    if (record) await api.startRecording(total + 3);
    await api.setDemoTour(true);
    tourStop.current = false;
    try {
      for (const [text, secs, act] of steps) {
        if (tourStop.current) break;
        setTour({ text, running: true });
        await act();
        for (let t = 0; t < secs * 10 && !tourStop.current; t++) await new Promise((r) => setTimeout(r, 100));
      }
    } finally {
      await api.setDemoTour(false).catch(() => {});
      setTour(null);
    }
    if (!record) return;
    const stopped = tourStop.current;
    const path = await api.stopRecording(!stopped);
    if (stopped) return;
    setTour({ text: `录屏已保存到「影片」：${path.split("/").pop()}（已在访达中选中）`, running: false });
    window.setTimeout(() => setTour(null), 8000);
  });

  async function doLock(reason?: string) {
    tourStop.current = true; // 自动演示中途锁定：脚本别再往下走了
    await api.lock();
    copyFreeUntil.current = 0; // 锁库就得重新验，不能留着免验窗口
    // 自动锁定可能正好撞上开着的复制闸门：挂着的请求给个结果，别让调用方永远等下去
    copyWaiting.current?.("cancel");
    copyWaiting.current = null;
    copyReturnTo.current = { name: "none" };
    setAccounts([]);
    setSessions([]);
    setOverlay({ name: "none" });
    // 从演示模式锁出来时，本机可能根本还没有真实账号库
    setScreen((await api.isInitialized()) ? { name: "unlock", reason } : { name: "register" });
  }

  // Settings 里点确认时已经带主密码调过 api.wipeVault 了，这里只管清理导航状态——
  // 跟 doLock 的道理一样，别在这儿再调一次 wipeVault，库都已经改名了，第二次调只会报错
  function doWipe() {
    copyFreeUntil.current = 0;
    setAccounts([]);
    setSessions([]);
    setOverlay({ name: "none" });
    setScreen({ name: "register" });
  }

  // ⌘K 快捷切换，Esc 收起浮层。表单页不接 ⌘K——它会跳走，正在填的内容就没了。
  useEffect(() => {
    function onKey(e: KeyboardEvent) {
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "k" && unlocked && screen.name !== "form") {
        e.preventDefault();
        setOverlay((o) => (o.name === "quickswitch" ? { name: "none" } : { name: "quickswitch" }));
      } else if (e.key === "Escape") {
        // 区域告警不给 Esc 关：页面已经被断了，随手关掉只剩一片白板，
        // 用户反而不知道发生了什么。必须走框里那两个明确的出口
        setOverlay((o) => {
          if (o.name === "alarm") return o;
          // 挂着的复制请求要有个结果，否则调用方的 await 永远回不来。
          // Esc 是"全都收起来"，所以不把闸门顶掉的那个浮层放回去
          copyWaiting.current?.("cancel");
          copyWaiting.current = null;
          copyReturnTo.current = { name: "none" };
          return { name: "none" };
        });
      }
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [unlocked, screen.name]);

  const overlays = (
    <>
      {overlay.name === "delete" && (
        <DeleteConfirm
          account={overlay.account}
          onCancel={() => setOverlay({ name: "none" })}
          onConfirm={() =>
            run(async () => {
              await api.deleteAccount(overlay.account.id);
              setOverlay({ name: "none" });
              await enterMain();
            })
          }
        />
      )}
      {overlay.name === "totp" && (
        <TotpCode account={overlay.account} onClose={() => setOverlay({ name: "none" })} onCopy={requestCopy} />
      )}
      {overlay.name === "copy" && (
        <CopyGate
          label={overlay.label}
          onCancel={() => settleCopy("cancel")}
          onPassed={async () => {
            copyFreeUntil.current = Date.now() + COPY_GRACE_MS;
            settleCopy(await copyFrom(overlay.text));
          }}
        />
      )}
      {overlay.name === "region" && (
        <RegionBlocked
          verdict={overlay.verdict}
          mode={overlay.mode}
          accountLabel={`${overlay.account.platform} · ${overlay.account.related_app || overlay.account.username}`}
          onCancel={() => setOverlay({ name: "none" })}
          onContinue={() => {
            const a = overlay.account;
            setOverlay({ name: "none" });
            void launch(a, true);
          }}
          onSettings={() => {
            setOverlay({ name: "none" });
            setScreen({ name: "sub", page: "settings" });
          }}
          onRecheck={async () => {
            const a = overlay.account;
            const s = await api.patrolRegion();
            setRegion(s);
            if (!api.regionNeedsHandling(s.verdict)) {
              setOverlay({ name: "none" });
              void launch(a, false);
            } else {
              setOverlay({ name: "region", account: a, verdict: s.verdict, mode: s.mode });
            }
          }}
        />
      )}
      {overlay.name === "alarm" && (
        <RegionAlarm
          verdict={overlay.verdict}
          cut={overlay.cut}
          onSettings={() => {
            setOverlay({ name: "none" });
            setScreen({ name: "sub", page: "settings" });
          }}
          onIgnore={() => setOverlay({ name: "none" })}
          onCut={() =>
            run(async () => {
              await api.cutSessions();
              setSessions(await api.listActiveSessions());
              setOverlay({ name: "alarm", verdict: overlay.verdict, cut: true });
            })
          }
          onRecheck={async () => {
            // 只有后端确认回到大陆才会真的接回页面，前端点不动这道闸
            const s = await api.recheckAndRestore();
            setRegion(s);
            setSessions(await api.listActiveSessions());
            if (api.regionNeedsHandling(s.verdict)) return false;
            setOverlay({ name: "none" });
            return true;
          }}
        />
      )}
      {overlay.name === "quickswitch" && (
        <QuickSwitch accounts={accounts} sessions={sessions} onClose={() => setOverlay({ name: "none" })} onPick={launch} />
      )}
      {err && (
        <div
          style={{
            position: "fixed",
            left: 24,
            bottom: 24,
            background: C.surface,
            border: `1px solid ${C.danger}`,
            borderRadius: 8,
            padding: "10px 14px",
            maxWidth: 420,
          }}
        >
          <ErrorLine text={err} />
          <span onClick={() => setErr("")} style={{ fontSize: 11, color: C.muted, cursor: "pointer" }}>
            关闭
          </span>
        </div>
      )}
    </>
  );

  const shell: SubPageShell = { sessionCount: sessions.length, onGo: goto, onLock: () => void doLock(), onHelp: () => setHelp(true) };

  const page = () => {
    switch (screen.name) {
      case "loading":
        return <div style={{ padding: 32, color: C.muted }}>加载中…</div>;

      case "register":
        return <Register onDone={enterMain} onDemo={enterDemo} onTour={runTour} />;

      case "unlock":
        return <Unlock onDone={enterMain} onDemo={enterDemo} onTour={runTour} reason={screen.reason} />;

      case "form":
        return (
          <AccountForm
            initial={screen.account}
            isNew={screen.isNew}
            copiedFrom={screen.copiedFrom}
            accounts={accounts}
            onCancel={() => void leaveForm(screen.back)}
            onSave={async (a) => {
              await api.saveAccount(a);
              await leaveForm(screen.back);
            }}
          />
        );

      case "sub":
        return (
          <>
            {screen.page === "settings" ? (
              <Settings shell={shell} onLocked={() => doLock()} onWiped={doWipe} onRegionChanged={setRegion} onAutoLockChanged={setAutoLockSecs} />
            ) : (
              <PlatformConfigScreen key={configVersion} shell={shell} accounts={accounts} onPick={pickFor} />
            )}
            {help && <AdaptHelp onClose={() => setHelp(false)} onImported={() => setConfigVersion((v) => v + 1)} />}
          </>
        );

      case "sessions":
        return <SessionsPage accounts={accounts} sessionCount={sessions.length} onGo={goto} onLock={() => doLock()} onOpen={launch}
          pickTarget={pickTarget} onPickHandled={() => setPickTarget(null)}
          onAccountsChanged={() => void refreshAccounts()}
          onAdd={() => setScreen({ name: "form", account: api.emptyAccount(), isNew: true, back: "sessions" })}
          onEdit={(account) => openForm(account, "sessions")} />;

      case "main":
        return (
          <Main
            accounts={accounts}
            sessions={sessions}
            onAdd={() => setScreen({ name: "form", account: api.emptyAccount(), isNew: true, back: "main" })}
            onEdit={(a) => openForm(a, "main")}
            onDuplicate={(a) => openForm(a, "main", true)}
            onDelete={(account) => setOverlay({ name: "delete", account })}
            onLaunch={launch}
            onTotp={(account) => setOverlay({ name: "totp", account })}
            onCopy={requestCopy}
            onLock={() => doLock()}
            onGo={goto}
          />
        );
    }
  };

  return (
    <AppShell demo={unlocked && api.inDemo()} tour={tour} onStopTour={() => { tourStop.current = true; }} region={region} checking={checkingRegion} notice={regionNotice} onCheck={() => void checkNetwork()} onDismiss={() => setRegionNotice("")}>
      {page()}
      {overlays}
    </AppShell>
  );
}
