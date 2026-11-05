// A person's pick of light or dark, before the page first paints (i18n/theme.ts keeps it).
try {
  var picked = localStorage.getItem('theme');
  if (picked === 'light' || picked === 'dark') document.documentElement.dataset.theme = picked;
} catch (e) {
  // Storage is off: the system's look.
}
