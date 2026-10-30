// The demo pages' words in the languages easi.run speaks: the browser's first that is one of them, which is the visitor's
// own (the admin starts a visit's Chrome in the language of the visitor's page), else English.

// English, here, so a page speaks it whatever becomes of the other languages' file. scripts/demo.py reads it as JSON.
const ENGLISH = {
  "takeover.title": "Acme · Sign in",
  "takeover.heading": "Sign in to Acme",
  "takeover.lead": "A fake sign-in page. Type any password and any 6-digit code.",
  "takeover.email": "Email",
  "takeover.password": "Password",
  "takeover.anyPassword": "Any password",
  "takeover.signIn": "Sign in",
  "takeover.codeHeading": "2-Step Verification",
  "takeover.codeLead": "Enter the 6-digit code from your authenticator app (any 6 digits work here).",
  "takeover.code": "Code",
  "takeover.verify": "Verify",
  "takeover.done": "Signed in ✓",
  "takeover.doneLead": "This is where the agent would take over again. Nothing you typed left this page.",
  "takeover.hint": "In easi.run, text of four characters or more you type or paste while you hold control reaches the agent as [redacted].",
  "drag.title": "Drag & drop · easi.run demo",
  "drag.heading": "Drag a file onto the page",
  "drag.lead": "Drag any file from your computer into the box. It stays in this browser.",
  "drag.zone": "Drop a file here",
  "drag.one": "✓ {name} received · {size}",
  "drag.many": "✓ {count} files received · {size}",
  "drag.hint": "Use sample files only. The file stays inside this isolated demo browser, which is deleted when your visit ends.",
  "upload.title": "Upload · easi.run demo",
  "upload.heading": "Upload a file",
  "upload.lead": "Press the button: your own computer's file picker opens.",
  "upload.choose": "Choose files",
  "upload.attached": "attached ✓",
  "upload.hint": "The file stays inside this isolated demo browser, which is deleted when your visit ends.",
  "download.title": "Download · easi.run demo",
  "download.heading": "Download a file",
  "download.lead": "Click a file: it lands in your own Downloads folder.",
  "download.button": "Download",
  "sound.title": "Sound · easi.run demo",
  "sound.heading": "Hear this browser",
  "sound.lead": "Press play: the sound comes from Chrome on my server, live, to your speakers.",
  "sound.play": "▶ Play the jingle",
  "sound.stop": "Stop",
  "sound.notes": [
    "C",
    "D",
    "E",
    "F",
    "G",
    "A",
    "B",
    "C"
  ],
  "sound.hint": "No sound? Unmute or raise the volume in the easi.run bar above the screen.",
  "typing.title": "Typing · easi.run demo",
  "typing.heading": "Type at full speed",
  "typing.lead": "Type the sentence. Every key travels from your keyboard to Chrome on my server.",
  "typing.label": "Your turn",
  "typing.placeholder": "Start typing…",
  "typing.speed": "words per minute",
  "typing.perWord": 5,
  "typing.accuracy": "accuracy",
  "typing.again": "New sentence",
  "typing.sentences": [
    "The quick brown fox jumps over the lazy agent.",
    "My browser runs on a server, but it feels local.",
    "You drive this browser from wherever you are."
  ],
  "typing.hint": "In easi.run, text of four characters or more you type while you hold control reaches the agent as [redacted].",
  "clipboard.title": "Copy & paste · easi.run demo",
  "clipboard.copyHeading": "Copy from the page",
  "clipboard.copyLead": "Press Copy, then paste anywhere on your computer.",
  "clipboard.copy": "Copy",
  "clipboard.copied": "Copied ✓",
  "clipboard.refused": "The browser refused the copy",
  "clipboard.pasteHeading": "Paste into the page",
  "clipboard.pasteLead": "Copy any text on your computer, click the box and paste (Ctrl+V or ⌘V).",
  "clipboard.paste": "Paste here…",
  "clipboard.pasted": "Pasted {count} characters from your computer ✓",
  "clipboard.hint": "In easi.run, text of four characters or more you paste while you hold control reaches the agent as [redacted]."
};
// The languages the pages speak, i18n/locales.ts's (scripts/demo.py keeps them in step with words.json).
const LOCALES = ['en', 'pt-BR', 'es', 'zh-CN', 'hi', 'fr', 'de', 'ja', 'ru', 'id', 'ko'];
const wanted = navigator.languages.find((tag) => LOCALES.includes(tag)) ?? 'en';
// The other languages (words.json), fetched only for one of them: none when the file fails or takes longer than 1.5 s,
// the page then in English, working well before demo.css shows it whatever happens.
const OTHERS = wanted === 'en' ? {} : await Promise.race([
  import('/shared/words.json', { with: { type: 'json' } }).then((m) => m.default, () => ({})),
  new Promise((resolve) => setTimeout(resolve, 1500, {})),
]);
const WORDS = { en: ENGLISH, ...OTHERS };

export const locale = Object.hasOwn(WORDS, wanted) ? wanted : 'en';

/** The word `key` in the page's language (English where it has none), each `{name}` in it filled from `params`. */
export function t(key, params = {}) {
  const word = WORDS[locale][key] ?? ENGLISH[key];
  return typeof word === 'string' ? word.replace(/\{(\w+)\}/g, (_, name) => params[name]) : word;
}

// The page's own words: the text of each element with `data-t`, the placeholder of each with `data-placeholder`.
for (const element of document.querySelectorAll('[data-t]')) element.textContent = t(element.dataset.t);
for (const element of document.querySelectorAll('[data-placeholder]')) element.placeholder = t(element.dataset.placeholder);
// demo.css shows the page once it speaks its language.
document.documentElement.lang = locale;
