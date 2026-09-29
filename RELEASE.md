# 发版手册

从改代码到用户点「检查更新」拿到新版，全流程在这里。

```bash
#python3 scripts/release.py
## 首次：创建环境
conda create -n sb python=3.12 
## 激活环境 （conda deactivate 退出环境）
conda activate sb
## 运行
python scripts/release.py
```

**就这一条命令。** 跑起来会问你要做什么，不用记参数。

---

## 打包环境

先说结论：

| 在哪台机器上 | 能打哪些包 |
|---|---|
| **Mac**（推荐，平时发版就在这台上） | **macOS + Windows 两个平台**。Windows 的用 MinGW 交叉编译 |
| **Windows 电脑** | **只能打 Windows 包**。macOS 包必须在 Mac 上打：要 Apple 的 SDK 和 `hdiutil`、`lipo`、`codesign` 这些只有 macOS 才有的工具，苹果也不允许在非苹果硬件上跑 macOS |

两边打出来的包**版本号都取自 `src-tauri/tauri.conf.json`**，安装包文件名里带着版本号（如 `Switchboard_0.1.8_x64-setup.exe`），
`release.py` 只认版本号跟当前一致的包。所以只要两边用的是同一个提交，版本就一致。

### Mac：打 macOS + Windows 两个平台

| 要装的 | 干什么用 | 实测版本 |
|---|---|---|
| Xcode 命令行工具 | clang、`make`、`lipo`，还有系统自带的 `perl`（编 OpenSSL 要用） | Apple clang 17 |
| Homebrew | 装下面几样 | — |
| Rust（rustup） | 编译后端 | 1.94 |
| Rust 目标 `aarch64-apple-darwin`、`x86_64-apple-darwin` | macOS universal 包（一个包同时含 Apple Silicon 和 Intel） | — |
| Rust 目标 `x86_64-pc-windows-gnu` | Windows 包 | — |
| `mingw-w64` | Windows 的 C 编译器（SQLCipher、OpenSSL 要编成 Windows 版），约 1.4 GB | GCC 16 |
| `makensis` | 把 Windows 程序打成安装包（NSIS） | 3.12 |
| Node.js 20 或更高 + pnpm | 前端构建 | Node 20、pnpm 10 |
| Python 3 | 跑 `release.py`（系统自带的 `python3` 或上面的 conda 环境都行） | 3.12+ |

一条条装（已经装过的会自动跳过）：

```bash
xcode-select --install
```

```bash
/bin/bash -c "$(curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh)"
```

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

```bash
rustup target add aarch64-apple-darwin x86_64-apple-darwin x86_64-pc-windows-gnu
```

```bash
brew install node pnpm mingw-w64 makensis
```

然后在项目目录里装前端依赖：

```bash
pnpm install
```

再按下一节「首次准备」放好签名私钥和 Gitee 令牌。

检查齐没齐：

```bash
xcode-select -p && rustc -V && rustup target list --installed && node -v && pnpm -v && python3 -V && x86_64-w64-mingw32-gcc --version | head -1 && makensis -VERSION && ls ~/.switchboard-keys/updater.key
```

每一项都有输出、`rustup target list` 里有那三个目标就对了。也可以直接跑 `release.py`：菜单顶部 `Windows ：` 那行没有「还没装交叉编译工具」的提示，就说明 Windows 那套齐了。

> 第一次打包要把 OpenSSL 现编三遍（Mac 两个架构 + Windows），十来分钟；之后改了代码再打，几分钟。

### Windows 电脑：只打 Windows 包

平时用不上，Mac 上就能打 Windows 包。只在 Mac 交叉编译出问题、或者想要 MSVC 原生编译的版本时用。需要 Windows 10/11（x64）。

| 要装的 | 干什么用 |
|---|---|
| Visual Studio 2022 生成工具，勾「使用 C++ 的桌面开发」 | MSVC 编译器、Windows SDK，还有 OpenSSL 构建要用的 `nmake.exe` |
| Rust（rustup，工具链 `stable-msvc`） | 编译后端 |
| Node.js LTS + pnpm | 前端构建 |
| Strawberry Perl | OpenSSL 的配置脚本是 Perl 写的 |
| Git | 拉代码 |
| NASM（可选） | 装了 OpenSSL 会带汇编优化，不装也能编 |
| WebView2 运行时 | Windows 10/11 自带，不用装 |

打包工具 NSIS 不用装，tauri 第一次打包时会自己下载。

**以管理员身份**打开 PowerShell，一条条执行：

```powershell
winget install --id Microsoft.VisualStudio.2022.BuildTools --override "--wait --passive --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"
```

```powershell
winget install --id Rustlang.Rustup
```

```powershell
winget install --id OpenJS.NodeJS.LTS
```

```powershell
winget install --id StrawberryPerl.StrawberryPerl
```

```powershell
winget install --id Git.Git
```

装完**关掉 PowerShell 重新打开**（让新装的程序进 PATH），再执行：

```powershell
rustup default stable-msvc
```

```powershell
npm install -g pnpm
```

拉代码，并切到 Mac 上**这次发版的同一个提交**（版本一致靠的就是这个）：

```powershell
git clone git@github.com:en-o/switchboard.git
```

```powershell
cd switchboard; git checkout <Mac 上 git rev-parse HEAD 看到的那个提交>; pnpm install
```

把 Mac 上的 `~/.switchboard-keys/updater.key` 拷到这台电脑的 `%USERPROFILE%\.switchboard-keys\updater.key`（U 盘、网盘都行，拷完从中转的地方删掉）。
**必须是同一把私钥**，换了老客户端就装不上更新了。

检查齐没齐：

```powershell
rustc -V; node -v; pnpm -v; where.exe perl; Test-Path "$HOME\.switchboard-keys\updater.key"
```

`where.exe perl` 的**第一行要是 `C:\Strawberry\perl\bin\perl.exe`**。要是排在前面的是 Git 自带的 perl（`...\Git\usr\bin\perl.exe`），
OpenSSL 会配置失败，把 Strawberry Perl 的路径在系统环境变量 PATH 里挪到前面。

打包、送回 Mac 发布的步骤见后面「Windows 版 → 备用：在 Windows 电脑上打」。

---

## 首次准备（只做一次）

### 1. 签名私钥

更新包必须签名，客户端才肯装。密钥应该已经在 `~/.switchboard-keys/updater.key`：

```bash
ls ~/.switchboard-keys/updater.key && echo "已有，跳过"
```

没有的话（换了台机器、或误删了）：

```bash
cargo tauri signer generate -w ~/.switchboard-keys/updater.key -f --password ""
```

> ⚠️ **换了密钥，已经装在用户机器上的老版本就再也收不到更新了。**
> 老版本内置的是旧公钥，新密钥签的包它一律拒装，只能让人手动重新下载安装。
> 这把私钥请单独备份（密码管理器 / 离线存储都行），别只躺在这一台机器上。
>
> 公钥写死在 `src-tauri/tauri.conf.json` 的 `plugins.updater.pubkey`，真要换密钥的话这里也得同步改。

### 2. Gitee 令牌

脚本第一次要上传时会问你要，当场去生成：

1. 打开 <https://gitee.com/personal_access_tokens>
   （或：Gitee 右上角头像 →「设置」→ 左侧「私人令牌」）
2. 点「生成新令牌」，权限**至少勾上 `projects`** —— 创建发行版、上传附件、写 `latest.json` 都要它
3. 输入 Gitee 登录密码确认
4. **令牌只在这一屏显示一次**，当场复制，粘给脚本

粘贴时不会回显（走 `getpass`，也不会进 shell 历史）。验证通过后脚本会问要不要记住，选是就存到 `~/.switchboard-keys/gitee.token`（权限 `0600`，在仓库外）。

| 想做的事 | 怎么办 |
|---|---|
| 换一个令牌 | `rm ~/.switchboard-keys/gitee.token` 再跑，会重新问 |
| 临时用别的令牌 | `GITEE_TOKEN=xxx python3 scripts/release.py` |

---

## 发布

```bash
python3 scripts/release.py
```

```
────────────────────────────────────────────────────
Switchboard 发布    版本 0.1.8  →  发行版 tag v0.1.8
macOS   ：Switchboard_0.1.8_universal.dmg（10 分钟前构建，含签名）
Windows ：Switchboard_0.1.8_x64-setup.exe（10 分钟前构建，含签名）
────────────────────────────────────────────────────
  1) 发布          重新构建并上传（默认 macOS + Windows，也可以只发一个）
  2) 只构建        两个平台都出包，不上传
  3) 直接上传      复用已有产物（现在有：Windows、macOS）
  4) 改版本号
  0) 退出
请选择 [1]：
```

顶部先把**当前版本、对应 tag、两个平台各自有没有产物、多久前构建的**摆出来，省得自己翻目录确认。

选 1 或 3 之后会问发哪些平台，**默认两个都发**：

```
发布哪些平台？
  1) macOS + Windows（默认）
  2) 只发 macOS
  3) 只发 Windows
```

| 选项 | 什么时候用 |
|---|---|
| **1 发布** | 正常发版。按选的平台重新构建，一起上传到同一个发行版、写进同一份 `latest.json` |
| **2 只构建** | 想先看看包对不对、或者只是要个 dmg / exe 发给别人试。两个平台都打 |
| **3 直接上传** | 刚构建过（比如先选 2 测过了），或者上传那步失败了要重试——**省掉重新构建**。选的平台必须有当前版本的产物 |
| **4 改版本号** | 只在想跳版本时用（如 0.1.5 → 0.2.0）。正常发版会自动进位 |

两个平台的包都要打的时候，任何一个没打出来就**什么都不发**，免得不知不觉只发了一半；想先发能打出来的那个，重跑时选「只发 macOS」或「只发 Windows」。

几处会打断你，都是故意的：

- **令牌在构建之前验。** 构建要好几分钟，不该等到最后才告诉你令牌不对。
- **真正上传前再确认一次。** 会列清楚每个平台传哪几个文件、传到哪个仓库，并提醒"所有老客户端都会收到这个更新"——往公开仓库推东西，值得多问一句。
- **只发一个平台时**，确认前会提醒另一个平台的清单条目会怎样（见「Windows 版 → 只发一个平台」），发完会问版本号要不要进位。

### 一次典型的发版

```
改代码 → 测 → 提交 → 跑脚本选 1 → 回车（两个平台）→ 确认 → 自查
```

想先装上试试再发：选 2 只构建 → 装 dmg / exe 试 → 再跑脚本选 3 直接上传。

版本号不用管，两个平台都发了之后脚本会自动 +1。

---

## 版本号：发完自动进位

取自 `src-tauri/tauri.conf.json` 的 `version`，发行版 tag 就是 `v` + 它。

**两个平台都发了之后，脚本会自动把修订号 +1**（0.1.0 → 0.1.1），工作区直接停在下一个版本。
正常发版完全不用管版本号，改完代码跑脚本选 1 就行。只发了一个平台时会先问一句要不要进位，见「Windows 版 → 只发一个平台」。

> 为什么要自动进位：在线更新靠版本号比大小决定"有没有新版"。忘了调高的话，
> 老客户端查更新会一直显示「已是最新」——**发了等于没发，而且不报错**，最难查。

### 进位后旧产物会被挡住

进位之后 `target/` 里的产物还是**旧版本**的，所以菜单会标出来，「直接上传」也不认它：

```
macOS   ：Switchboard_0.1.0_universal.dmg（24 分钟前构建，含签名，属于 0.1.0，**与当前版本不符**）
Windows ：还没有 0.1.1 的安装包
  3) 直接上传      不可用：没有 0.1.1 的产物，请选 1 或 2
```

不这么挡的话，会用 `v0.1.1` 的 tag 传一个 `Switchboard_0.1.0_universal.dmg`，名实不符。Windows 安装包同理，只认文件名里是当前版本的。

### 想跳版本就手动改

比如 0.1.5 直接发 0.2.0，用菜单选 4，有两道护栏：

| 输入 | 结果 |
|---|---|
| `abc` | 格式要是 x.y.z，比如 0.1.1 |
| `0.0.9` | 必须高于当前的，否则更新推不出去 |
| `0.2.0` | ✅ 已改成 0.2.0 |

---

## 发完自查

```bash
curl -sL https://gitee.com/etn/switchboard_store/raw/master/latest.json
```

三件事要对：

- `version` 是刚发的新版号
- `platforms.darwin-aarch64.url` 和 `darwin-x86_64.url` 能下载到东西；这次带了 Windows 的话，`windows-x86_64.url` 也要能下载
- `signature` 非空

都对的话，老版本 App 在「设置 → 检查更新」就能看到它。

发行版页面：<https://gitee.com/etn/switchboard_store/releases>

---

## 在线更新是怎么工作的

```
App「检查更新」
   └─ GET https://gitee.com/etn/switchboard_store/raw/master/latest.json
        └─ 比较 version，更高才算有新版
             └─ 下载 platforms.darwin-*.url 指向的 .app.tar.gz
                  └─ 用内置公钥验 signature ── 不通过就拒装
                       └─ 替换自身 → 重启
```

几个要点：

- **端点是仓库里的 `latest.json`，不是发行版附件。** Gitee 附件的 URL 事先猜不出来（形如 `/attach_files/<随机 id>/download/<名字>`），没法当固定端点。所以脚本先传附件、**回读真实地址**，再把地址写进 `latest.json`。
- **验签是硬门槛。** 别人就算伪造下载链接、甚至改掉 `latest.json`，没有私钥签的包客户端也装不上。
- `latest.json` 是脚本通过 Gitee API 直接写进仓库的，**源码不会被推上去**。

---

## 产物与代码保护

打出来的 `.app` 里**只有 3 个文件**：`Info.plist`、可执行文件、图标。没有任何散落的源码。

| 检查 | 结果 |
|---|---|
| `.app` 内文件数 | 3 |
| `grep` 二进制找 `useState` / `createRoot` / 界面文案 | 全部 0 命中 |
| Rust 符号表 | 已 strip（`nm` 只剩系统库外部引用） |

- 前端被 **brotli 压缩后嵌进二进制**，不是可以直接打开的文件
- 发版前会过一道 JS 混淆（`scripts/obfuscate.mjs`，挂在 `pnpm build:app` 上，`pnpm dev` 不受影响），字符串进 base64 数组、中文转 `\uXXXX`
- Rust 侧 `strip = "symbols"` + LTO + `codegen-units = 1`

**这不是加密，只是抬高门槛。** 有决心的人照样能把资源段解出来、把二进制反汇编。真正的秘密是用户的账号密码和 TOTP 密钥，它们在 SQLCipher 库里由主密码保护，跟代码混淆是两件事。

---

## 为什么不用 Docker

macOS 的 `.app`/`.dmg` **只能在 macOS 上打**：要 Apple SDK，要 `hdiutil`/`codesign`/`sips` 这些只存在于 macOS 的工具，Apple 的许可也不允许在非苹果硬件的容器里跑 macOS。

所以这里的"多架构"是本机交叉编译成 **universal 包**——一个包里同时装 arm64 和 x86_64，Intel 和 Apple Silicon 都能直接跑。验证方式：

```bash
lipo -archs src-tauri/target/universal-apple-darwin/release/bundle/macos/Switchboard.app/Contents/MacOS/switchboard
# x86_64 arm64
```

Windows 包也不用 Docker：Mac 上装一套 MinGW 就能交叉编译出来，见「Windows 版」。

---

## Windows 版

Windows 安装包**也在这台 Mac 上打**：用 MinGW 交叉编译（`x86_64-pc-windows-gnu`），NSIS 打安装包。环境见「打包环境 → Mac」。
`release.py` 选 1 / 2 时两个平台一起打，版本号天然一致（两边都读 `tauri.conf.json`）。

为什么不走 Tauri 文档里的 MSVC 交叉编译（cargo-xwin）：账号库的 SQLCipher 要现编 OpenSSL，OpenSSL 的 MSVC 构建非得用
Windows 上的 `nmake.exe`，Mac 上没有；MinGW 那条走普通 `make`，Mac 上能编。实测产物只依赖 Windows 自带的系统 DLL，
外加 `WebView2Loader.dll`（MinGW 版要动态加载它，tauri 会自动打进安装包）。

> ⚠️ 打出来的包**还没在 Windows 真机上跑过**。Tauri 对交叉编译的说法是"实验性"，第一次发之前，
> 先选 2 只构建，把 `src-tauri/target/x86_64-pc-windows-gnu/release/bundle/nsis/` 下的安装包拿到一台 Windows 上按「上线前实测」走一遍，
> 没问题再选 3 直接上传。

发布时传进发行版的是 `_x64-setup.exe`，`latest.json` 里对应 `windows-x86_64`，Windows 的在线更新装的就是这个安装包。

**没有代码签名证书**时，Windows 上第一次运行会弹「Windows 已保护你的电脑」，点「更多信息 → 仍要运行」。
要去掉得买代码签名证书，在 `tauri.conf.json` 的 `bundle.windows` 里配 `signCommand`（Mac 上可以用 `osslsigncode`）。
构建日志里那句 "Signing ... skipping signing the installer" 说的就是这个，跟在线更新的签名无关——更新签名（`.sig`）是正常生成的。

### 只发一个平台

`latest.json` 只有一个顶层版本号，所以：

- **另一个平台的条目版本也一样**（比如先发了 0.1.8 的 macOS，现在补 0.1.8 的 Windows）：保留，两个平台都能更新
- **另一个平台的条目是旧版本**：去掉。不然那个平台的客户端会没完没了地"更新"到旧包（装完一看版本号还是旧的，下次又提示）。
  代价是那个平台的用户查更新会提示失败，直到它也发了这个版本

所以只发一个平台之后，脚本会问**版本号要不要进位**：

- 之后还要给**这个版本**补另一个平台：选不进位（`n`），补的时候跑脚本选 1 或 3 →「只发另一个」，会传进同一个发行版
- 不补了：进位（直接回车）。不补又不进位的话，下次发版老客户端会一直显示「已是最新」

### 备用：在 Windows 电脑上打

环境见「打包环境 → Windows 电脑」。在那台电脑的 PowerShell 里、项目目录下：

```powershell
$env:TAURI_SIGNING_PRIVATE_KEY = Get-Content -Raw "$HOME\.switchboard-keys\updater.key"
```

```powershell
$env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD = ""
```

```powershell
pnpm tauri build --bundles nsis
```

产物在 `src-tauri\target\release\bundle\nsis\`：`Switchboard_<版本>_x64-setup.exe` 和同名的 `.sig`，两个都要。
拷回 Mac 的 `src-tauri/target/windows-nsis/`（没有这个目录就建一个），然后在 Mac 上：

- **两个平台一起发**：先选 2 只构建（把 macOS 包打好；它也会顺手交叉编译一份 Windows 包）→ **再**把 Windows 电脑打的拷进来 → 选 3 直接上传 → 回车（两个平台）。
  两份 Windows 包同时在的时候，脚本用**更新的**那个，所以拷的动作要放在选 2 之后
- **只发 Windows**：选 3 直接上传 →「只发 Windows」

脚本只认文件名里版本号跟当前 `tauri.conf.json` 一致的安装包，拷错了版本会被当成没有。

### 和 macOS 的区别

| 项 | macOS | Windows |
|---|---|---|
| 数据目录 | `~/Library/Application Support/com.switchboard.app/` | `%APPDATA%\com.switchboard.app\` |
| 各账号的登录数据 | `~/Library/WebKit/com.switchboard.app/WebsiteDataStore/<账号 id>` | 数据目录下的 `profiles\<账号 id>`（每个账号一个 WebView2 数据目录） |
| 锁定时抹登录数据 | 立刻删掉 | WebView2 关掉后还会占着文件几秒，删不掉就后台隔几秒重试（`session::retry_remove`） |
| 剪贴板 30 秒清空 | `pbpaste` / `pbcopy` | 隐藏窗口的 PowerShell |
| 忘记主密码的重置命令 | 终端里跑 `switchboard reset` | 一条 PowerShell 命令（正式版是 GUI 程序，命令行里跑它看不到任何输出） |
| 系统浏览器打开 | `open` | `rundll32 url.dll,FileProtocolHandler`（不经过 cmd，地址里的 `&` 不会被当命令执行） |
| 下载文件名 | 只清 `/ \ :` | 再清 `< > " \| ? *`、末尾的点和空格，`CON`、`NUL` 这类设备名前面加 `_` |
| 自动演示录屏 | 有 | 没有（选项不显示） |
| 识别二维码图片 | 有（系统自带的 CoreImage） | 没有（按钮不显示），粘贴 otpauth:// 链接照常能用 |
| 快捷键写法 | ⌘K、⌘V | Ctrl+K、Ctrl+V（按键本来就两边都认） |

### 上线前实测

1. 装好后注册、解锁、新增账号、一键登录，自动填充能填上（阿里云那种登录框在 iframe 里的也要试）
2. 两个账号同时开，登录态互不串；锁定再解锁，cookie 登录的站还是登录状态
3. 锁定后等十几秒，看 `%APPDATA%\com.switchboard.app\profiles\` 下对应账号的目录被删掉了
4. 页面里下载一个文件，点提示条能在资源管理器里选中它
5. 复制密码，30 秒后剪贴板被清空；这期间复制了别的就不清
6. 「系统浏览器」能打开当前地址；开着代理时会被拦并说明原因
7. 登录页「忘记主密码」里的命令在 PowerShell 里能执行，重开 App 回到创建主账号
8. 进演示模式、锁定退出、马上再进一次，能进去
9. 「设置 → 检查更新」能看到新版本并装上（需要 `latest.json` 里有 `windows-x86_64`）

---

## 出错了怎么办

| 报错 / 现象 | 原因 | 怎么修 |
|---|---|---|
| `令牌无效或权限不足` | 令牌错了，或生成时没勾 `projects` | `rm ~/.switchboard-keys/gitee.token` 重跑，重新生成时勾上 `projects` |
| `找不到签名私钥` | `~/.switchboard-keys/updater.key` 不在 | 见上面「签名私钥」。**注意换密钥会让老版本收不到更新** |
| `A public key has been found, but no private key` | 签名环境变量没传进去 | 脚本已处理（传的是私钥**内容**而非路径）。手动跑 `tauri build` 时要 `export TAURI_SIGNING_PRIVATE_KEY=$(cat ~/.switchboard-keys/updater.key)` |
| `error running bundle_dmg.sh` | **偶发**。这步靠 AppleScript 指挥 Finder 摆图标，对时序敏感（报 `-10006` / `Can't get disk (-1728)`）。残留的同名卷也会触发 | **脚本已自己扛**：构建前清理残留卷；失败自动重试一次；仍失败就改用 `hdiutil` 出朴素 DMG（无自定义背景，但能装）。手动跑 `pnpm tauri build` 撞上的话，`hdiutil detach /dev/diskN -force` 卸掉残留卷再试 |
| 用户点「检查更新」永远显示"已是最新" | 版本号没调高（正常情况下脚本会自动进位，除非上次发布中途失败） | 菜单选 4 调高版本，重新发 |
| 更新下载完装不上 | 签名对不上（换过密钥、或包被改过） | 确认用的是当初那把私钥；实在不行让用户手动下 dmg 重装 |
| 上传成功但 `latest.json` 没更新 | 令牌缺仓库写权限 | 确认令牌勾了 `projects` |
| `AttributeError: 'NoneType' object has no attribute 'get'` | Gitee 查**不存在的 tag** 时返回 `HTTP 200` + 响应体 `null`（不是 404），脚本当字典用就崩了。首次发版必撞 | 已修：`api()` 把所有非字典响应归一成字典 |
| **启动台里冒出好几个 Switchboard** | 打 DMG 会临时挂载卷，里面的 `.app` 被注册进 LaunchServices；卷卸了条目还在。攒几次就显示成装了很多份 | 脚本每次构建完自动注销。手动清：见下 |

---

## DMG 那步为什么会偶发失败

`bundle_dmg.sh` 出完镜像后，会调 AppleScript 让 **Finder** 把图标摆到配置的位置、贴上背景图。
这一步对时序敏感——脚本 sleep 2 秒就赌 Finder 已经挂载并索引完那个卷。
机器忙、或者上次的卷还挂着，就会报：

```
execution error: “Finder”遇到一个错误：不能将“item "Switchboard.app" of disk "dmg.XXXX"”设置为“{180, 248}”。 (-10006)
Failed running AppleScript
```

**不是代码问题，重试通常就过。** 所以 `release.py` 里做了三层：

1. 构建**前**清理残留的 `dmg.*` / `Switchboard` 卷
2. 失败且确认是 DMG 那步 → 清理后**自动重试一次**
3. 还失败 → 只出 `.app`，再用 `hdiutil` 自己出一个**朴素 DMG**（没有自定义背景和图标位置，但能挂载、能拖进应用程序）

第 3 层是为了**不让一个 Finder 的时序问题卡住发版**。真走到这一层，产物里的 DMG 会少那张背景图，其余一切照常。

## 启动台里出现多个 Switchboard

**只有 `/Applications/Switchboard.app` 是真装的**，其余都是 LaunchServices 注册表里的残留条目：

- 每次打 DMG 会临时挂一个卷（`/Volumes/dmg.XXXXX`），里面那个 `.app` 会被注册进去，卷卸载后条目不会自动清
- 直接 `open` 过 `target/.../bundle/macos/Switchboard.app`（比如冒烟测试）也会把构建产物注册进去

`release.py` 每次构建完会自动注销这些条目并重启 Dock。手动清理：

```bash
LS=/System/Library/Frameworks/CoreServices.framework/Versions/A/Frameworks/LaunchServices.framework/Versions/A/Support/lsregister
"$LS" -dump | grep -oE "/[^ ]*Switchboard\.app" | sort -u | grep -v "^/Applications/" | while read -r p; do "$LS" -u "$p"; done
killall Dock
```

只注销注册，不删文件。清完 `"$LS" -dump | grep -oE "/[^ ]*Switchboard\.app" | sort -u` 应该只剩 `/Applications/Switchboard.app` 一个。

## 相关文件

| 路径 | 作用 |
|---|---|
| `scripts/release.py` | 发布脚本，交互式菜单，macOS + Windows 一起发 |
| `scripts/obfuscate.mjs` | 打包时的 JS 混淆，挂在 `pnpm build:app` |
| `src-tauri/tauri.conf.json` | `version` 版本号、`plugins.updater` 端点和公钥、`bundle` 打包配置 |
| `~/.switchboard-keys/updater.key` | 签名私钥，**不在仓库里，丢了发不了更新** |
| `~/.switchboard-keys/gitee.token` | Gitee 令牌，权限 0600 |
| `src-tauri/target/universal-apple-darwin/release/bundle/` | macOS 产物（`.dmg`、`.app.tar.gz` + `.sig`） |
| `src-tauri/target/x86_64-pc-windows-gnu/release/bundle/nsis/` | Mac 上交叉编译出来的 Windows 安装包（`_x64-setup.exe` + `.sig`） |
| `src-tauri/target/windows-nsis/` | 在 Windows 电脑上打好、拷回来的安装包放这里 |
