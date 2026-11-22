// What `inspect_page` reads of a page: its elements a selector matches, in an isolated world the page cannot see, and
// never the value of a password field. `q` is {selector, html, limit, text, attribute}, the last two how much of each
// to send, in characters (the admin keeps less, once it took out what a person typed, and counts characters too: a
// cut that counted UTF-16 units would fall short of it in a text with characters of two units, and leave the start of
// what a person typed past the admin's cut uncut); the answer is at most BUDGET bytes of JSON.
(function (q) {
  const BUDGET = 200000;
  const size = (s) => new TextEncoder().encode(s).length;
  // The first `most` characters of `s`, none of them halved, and no half of one from the page itself: a lone half
  // has no place in the JSON this answers with.
  const first = (s, most) => {
    let at = 0;
    for (let n = 0; n < most && at < s.length; n++) at += s.codePointAt(at) > 0xffff ? 2 : 1;
    return s.slice(0, at).toWellFormed();
  };
  const secret = (el) => el.localName === 'input' && (el.getAttribute('type') || '').toLowerCase() === 'password';
  const html = (el) => {
    const copy = el.cloneNode(true);
    const inputs = copy.querySelectorAll ? [...copy.querySelectorAll('input')] : [];
    for (const input of [copy, ...inputs]) if (input.localName === 'input' && secret(input)) input.removeAttribute('value');
    return copy.outerHTML;
  };
  const out = {
    visible: document.visibilityState === 'visible',
    focused: document.hasFocus(),
    url: location.href.toWellFormed(),
    title: document.title.toWellFormed(),
    // The HTTP status the document came with, when the browser says it (none for a page it made itself).
    status: (performance.getEntriesByType('navigation')[0] || {}).responseStatus || null,
    count: 0,
    truncated: false,
    elements: [],
  };
  // Asked only where the page is (whether it shows, for a navigation or a script): no element is looked for.
  if (q.limit === 0) return JSON.stringify(out);
  let found;
  try {
    found = document.querySelectorAll(q.selector);
  } catch {
    return JSON.stringify({ ...out, error: 'not a valid CSS selector' });
  }
  out.count = found.length;
  let used = size(JSON.stringify(out));
  for (const el of found) {
    if (out.elements.length >= q.limit) {
      out.truncated = true;
      break;
    }
    const attributes = {};
    for (const a of el.attributes) {
      if (a.name === 'value' && secret(el)) continue;
      attributes[a.name.toWellFormed()] = first(a.value, q.attribute);
    }
    const element = {
      tag: el.localName.toWellFormed(),
      attributes,
      text: first(el.innerText || el.textContent || '', q.text),
      shown: el.checkVisibility ? el.checkVisibility() : true,
    };
    if (q.html) element.html = html(el).toWellFormed();
    const cost = size(JSON.stringify(element));
    if (used + cost > BUDGET) {
      out.truncated = true;
      break;
    }
    used += cost;
    out.elements.push(element);
  }
  return JSON.stringify(out);
})
