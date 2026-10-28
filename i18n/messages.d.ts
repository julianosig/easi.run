// The languages i18n/vite.ts serves, one module each (each package's tsconfig includes this file).
declare module 'virtual:messages/*' {
  const catalog: import('./format.ts').Catalog;
  export default catalog;
}
// The site's docs keep theirs apart (site/vite.config.ts).
declare module 'virtual:docs-messages/*' {
  const catalog: import('./format.ts').Catalog;
  export default catalog;
}
