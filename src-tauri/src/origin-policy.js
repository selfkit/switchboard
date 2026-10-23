// Rust 与两个网页代理共用的显式来源策略；列表由构建时嵌入。
window.__sbOriginAllowed = (function (groups) {
  return function (a, b) {
    try { a = new URL(a); b = new URL(b); } catch (_) { return false; }
    if (!['http:', 'https:'].includes(a.protocol) || !['http:', 'https:'].includes(b.protocol)) return false;
    return a.origin === b.origin || groups.some(function (g) {
      return g.indexOf(a.origin) !== -1 && g.indexOf(b.origin) !== -1;
    });
  };
})(__SB_ORIGIN_GROUPS__);
