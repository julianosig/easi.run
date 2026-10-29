// A file's size as the demo pages show it, in binary units.
export const formatSize = (n) => (n > 1048576 ? `${(n / 1048576).toFixed(1)} MiB` : `${Math.max(1, Math.round(n / 1024))} KiB`);
