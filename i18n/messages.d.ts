// The languages i18n/vite.ts serves, one module each, and the table loading each but English (each package's tsconfig
// includes this file).
declare module 'virtual:messages/*' {
  const catalog: import('./format.ts').Catalog;
  export default catalog;
}
declare module 'virtual:messages' {
  type Catalog = import('./format.ts').Catalog;
  export const OTHERS: Record<Exclude<import('./locales.ts').Locale, 'en'>, () => Promise<{ default: Catalog }>>;
}
// The site's docs keep theirs apart (site/vite.config.ts).
declare module 'virtual:docs-messages/*' {
  const catalog: import('./format.ts').Catalog;
  export default catalog;
}
declare module 'virtual:docs-messages' {
  type Catalog = import('./format.ts').Catalog;
  export const OTHERS: Record<Exclude<import('./locales.ts').Locale, 'en'>, () => Promise<{ default: Catalog }>>;
}
