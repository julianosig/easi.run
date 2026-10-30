import { locale } from '/shared/words.js';

// A file's size as the demo pages show it, in binary units, its number written as the page's language writes it.
export function formatSize(n) {
  const [value, unit, digits] = n > 1048576 ? [n / 1048576, 'MiB', 1] : [Math.max(1, Math.round(n / 1024)), 'KiB', 0];
  return `${value.toLocaleString(locale, { minimumFractionDigits: digits, maximumFractionDigits: digits })} ${unit}`;
}
