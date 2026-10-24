// Motion runs only while the tab is visible.

export function followVisibility() {
  const sync = () => {
    if (document.hidden) document.documentElement.dataset.hidden = '';
    else delete document.documentElement.dataset.hidden;
  };
  sync();
  document.addEventListener('visibilitychange', sync);
}
