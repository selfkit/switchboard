// Both native and injected code use the same bundled PSL, including private hosting suffixes.
(function (rules) {
  const exact = new Set(), wildcard = new Set(), exception = new Set();
  for (const rule of rules) {
    if (rule.startsWith('!')) exception.add(rule.slice(1));
    else if (rule.startsWith('*.')) wildcard.add(rule.slice(2));
    else exact.add(rule);
  }
  function registeredDomain(host) {
    if (host.endsWith('.')) host = host.slice(0, -1);
    if (host.startsWith('[') || /^[\d.]+$/.test(host)) return null;
    const labels = host.split('.');
    if (labels.some(label => !label)) return null;
    let suffixLength = 1;
    for (let i = 0; i < labels.length; i++) {
      const suffix = labels.slice(i).join('.');
      if (exception.has(suffix)) return suffix;
      if (exact.has(suffix)) suffixLength = Math.max(suffixLength, labels.length - i);
      if (i > 0 && wildcard.has(suffix)) suffixLength = Math.max(suffixLength, labels.length - i + 1);
    }
    return labels.length > suffixLength ? labels.slice(-suffixLength - 1).join('.') : null;
  }
  function allowed(a, b) {
    try { a = new URL(a); b = new URL(b); } catch (_) { return false; }
    if (!['http:', 'https:'].includes(a.protocol) || a.protocol !== b.protocol) return false;
    if (a.origin === b.origin) return true;
    const site = registeredDomain(a.hostname);
    return site !== null && site === registeredDomain(b.hostname);
  }
  window.__sbOriginAllowed = allowed;
})(__SB_PUBLIC_SUFFIX_RULES__);
