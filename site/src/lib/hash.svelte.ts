// The address's fragment as it changes (the section on screen), for a link to the same place in another language.
// Read in the browser only: the page renders at its build without one.

/** The fragment, from a component's setup. */
export function currentHash(): { readonly value: string } {
  let hash = $state('');
  $effect(() => {
    const follow = () => (hash = location.hash);
    follow();
    window.addEventListener('hashchange', follow);
    return () => window.removeEventListener('hashchange', follow);
  });
  return {
    get value() {
      return hash;
    },
  };
}
