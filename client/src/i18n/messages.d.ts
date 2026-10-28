// The languages vite.config.ts's `messages` plugin serves, one module each.
declare module 'virtual:messages/*' {
  const catalog: import('./format.ts').Catalog;
  export default catalog;
}
