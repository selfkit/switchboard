# 登录站点边界

正常登录子域自动兼容。例如 `https://admin.example.com` 跳到 `https://login.example.com/auth`，或在 iframe 中嵌入该登录页，都不需要名单配置或额外确认。同一站点的不同端口也兼容。

站点采用“协议 + 可注册域名”判定。公共后缀表同时包含 ICANN 和 PRIVATE 部分，所以 `example.com.cn`、`example.co.uk` 的子域正常互通，而 `a.github.io` 与 `b.github.io`、不同 appspot.com/vercel.app 租户保持隔离。IP、localhost 和没有可注册域名的主机按完整来源比较；HTTPS 凭据不会降级投递到 HTTP。

判定由原生后端和网页代理共同执行。原生侧确认当前页面属于同站后，将填写脚本限定到本次已检查的精确来源，防止校验与执行之间跳到别站。登录重定向无需保持入口路径不变；自动填写继续排除新密码框、已有输入、只读/禁用控件及多个可见密码框的表单，并保留填写时限和 SPA 任务失效检查。

## 数据与许可

- 官方数据来源：[Public Suffix List](https://publicsuffix.org/list/)，运行时不联网下载。
- 本次快照：`2026-10-22_08-36-41_UTC`，commit `8437e5262f13305f94087abe699b4fe2be1d1de8`。
- [随应用携带的列表](/Users/macbookpro/Desktop/tan/code/me/switchboard/src-tauri/resources/public_suffix_list.dat)：保留官方注释和 MPL-2.0 许可声明，仅将规则主机名规范化为 ASCII/Punycode，确保 Rust 和 JS 使用同一份规则。
- Rust 使用 publicsuffix 解析库；网页代理按官方的最长匹配、通配符和例外规则解析相同数据。两侧均执行 78 项官方测试向量，另覆盖同站登录、私有后缀、协议、端口、IP 和国际化域名。
- [官方测试原文](/Users/macbookpro/Desktop/tan/code/me/switchboard/scripts/fixtures/public-suffix-tests.txt) 来自同一 commit 的 `tests/test_psl.txt`，其原文声明为 CC0/公共领域；JSON 测试向量由该文件生成。

## 开发维护

应用用户无需维护名单。更新数据时，由开发者下载官方 `https://publicsuffix.org/list/public_suffix_list.dat`，以及其 COMMIT 对应的官方 `tests/test_psl.txt`，再执行：

```sh
node scripts/update-psl.cjs /path/to/public_suffix_list.dat /path/to/test_psl.txt
pnpm check:origins
pnpm check:agent
pnpm check:bindings
cargo test --offline --manifest-path src-tauri/Cargo.toml
```

更新本页的快照标识并检查数据差异。保留 PRIVATE 部分，不能退回按域名末尾两段推断站点。
