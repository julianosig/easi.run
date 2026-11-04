// What `inspect_page` reads of a page: its elements a selector matches, in an isolated world the page cannot see, and
// never the value of a password field. `q` is {selector, html, limit, text, attribute}, the last two how much of each
// to send (the admin keeps less, once it took out what a person typed); the answer is at most BUDGET bytes of JSON.
(function (q) {
  const BUDGET = 200000;
  const size = (s) => new TextEncoder().encode(s).length;
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
    url: location.href,
    title: document.title,
    count: 0,
    truncated: false,
    elements: [],
  };
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
      attributes[a.name] = a.value.slice(0, q.attribute);
    }
    const element = {
      tag: el.localName,
      attributes,
      text: (el.innerText || el.textContent || '').slice(0, q.text),
      shown: el.checkVisibility ? el.checkVisibility() : true,
    };
    if (q.html) element.html = html(el);
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
