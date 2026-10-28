// The languages i18n/vite.ts serves, one module each (each package's tsconfig includes this file).
declare module 'virtual:messages/*' {
  const catalog: import('./format.ts').Catalog;
  export default catalog;
}
