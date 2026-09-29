// 极简 .dc.html 运行时：支持 {{表达式}}、<sc-for>、<sc-if>，供本地静态预览使用。
class DCLogic {}

function dcGet(expr, scope) {
  expr = expr.replace(/[{}]/g, '').trim();
  if (expr === 'true') return true;
  if (expr === 'false') return false;
  return expr.split('.').reduce((o, k) => (o == null ? o : o[k]), scope);
}

function dcText(s, scope) {
  return s.replace(/\{\{([^}]+)\}\}/g, (_, e) => {
    const v = dcGet(e, scope);
    return v == null ? '' : v;
  });
}

function dcRender(node, scope) {
  for (const n of [...node.childNodes]) {
    if (n.nodeType === 3) {
      if (n.textContent.includes('{{')) n.textContent = dcText(n.textContent, scope);
      continue;
    }
    if (n.nodeType !== 1) continue;
    const tag = n.tagName.toLowerCase();

    if (tag === 'sc-for') {
      const list = dcGet(n.getAttribute('list'), scope) || [];
      const as = n.getAttribute('as');
      const frag = document.createDocumentFragment();
      for (const item of list) {
        const tpl = n.cloneNode(true);
        dcRender(tpl, { ...scope, [as]: item });
        frag.append(...[...tpl.childNodes]);
      }
      n.replaceWith(frag);
      continue;
    }

    if (tag === 'sc-if') {
      if (dcGet(n.getAttribute('value'), scope)) {
        dcRender(n, scope);
        n.replaceWith(...[...n.childNodes]);
      } else {
        n.remove();
      }
      continue;
    }

    for (const a of [...n.attributes]) {
      if (a.value.includes('{{')) n.setAttribute(a.name, dcText(a.value, scope));
    }
    dcRender(n, scope);
  }
}

addEventListener('DOMContentLoaded', () => {
  const root = document.querySelector('x-dc') || document.body;
  let vals = {};
  try {
    if (typeof Component !== 'undefined') vals = new Component().renderVals() || {};
  } catch (e) {
    console.error('renderVals failed', e);
  }
  dcRender(root, vals);
});
