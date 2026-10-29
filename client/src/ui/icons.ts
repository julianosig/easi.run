// Every line icon of the admin page, by what it means (24-unit paths, drawn as strokes by Icon.svelte). One meaning,
// one icon: an icon is never reused for something else, so a person reads the same shape the same way everywhere.

export const ICONS = {
  // The pages.
  sessions: 'M4 5h16v14H4zM4 9h16M7 7h.01M10 7h.01',
  agents: 'M4 3l7 17 2.5-7.5L21 10zM14 14l5 5',
  proxies: 'M12 21s-6.5-5.6-6.5-10.5a6.5 6.5 0 0 1 13 0C18.5 15.4 12 21 12 21zM12 12.5a2 2 0 1 0 0-4 2 2 0 0 0 0 4z',
  security: 'M12 3l7 3v5c0 4.5-3 8.4-7 10-4-1.6-7-5.5-7-10V6zM9 12l2 2 4-4',

  // What a button does.
  add: 'M12 5v14M5 12h14',
  search: 'M11 18a7 7 0 1 0 0-14 7 7 0 0 0 0 14zM20 20l-4-4',
  edit: 'M21 4h-7M10 4H3M21 12h-9M8 12H3M21 20h-5M12 20H3M14 2v4M8 10v4M16 18v4',
  delete: 'M3 6h18M8 6V4h8v2M19 6l-1 14H6L5 6M10 11v6M14 11v6',
  close: 'M18 6 6 18M6 6l12 12',
  start: 'M8 5.5v13l10.5-6.5z',
  watch: 'M2.5 12s3.5-6.5 9.5-6.5 9.5 6.5 9.5 6.5-3.5 6.5-9.5 6.5S2.5 12 2.5 12zM12 15a3 3 0 1 0 0-6 3 3 0 0 0 0 6z',
  unlock: 'M6 11h12v9H6zM9 11V8a3 3 0 0 1 5.8-1.1',
  connect: 'M9 7V3M15 7V3M7 7h10v4a5 5 0 0 1-10 0zM12 16v5',
  retry: 'M21 12a9 9 0 1 1-2.6-6.4L21 8M21 3v5h-5',
  turnOff: 'M12 3v8M6.4 6.4a8 8 0 1 0 11.2 0',
  replace: 'M4 9h13l-3-3M20 15H7l3 3',
  logIn: 'M10 4H6a2 2 0 0 0-2 2v12a2 2 0 0 0 2 2h4M14 8l4 4-4 4M18 12H8',
  logOut: 'M14 4h4a2 2 0 0 1 2 2v12a2 2 0 0 1-2 2h-4M10 8l-4 4 4 4M6 12h10',
  check: 'm5 12 5 5 9-10',
  copy: 'M9 9h11v11H9zM5 15H4V4h11v1',
  download: 'M12 4v11M7 10l5 5 5-5M5 20h14',
  upload: 'M12 20V9M7 14l5-5 5 5M5 4h14',
  next: 'M5 12h14M13 6l6 6-6 6',

  // What a thing is.
  website: 'M12 3a9 9 0 1 0 0 18 9 9 0 0 0 0-18zM3.5 9h17M3.5 15h17M12 3c2.4 2.6 3.6 5.6 3.6 9s-1.2 6.4-3.6 9c-2.4-2.6-3.6-5.6-3.6-9S9.6 5.6 12 3z',
  persistent: 'M6 4h12v16l-6-4-6 4z',
  name: 'M4 12V5h7l9 9-7 7zM8.5 8.5h.01',
  devtools: 'M8 8l-4 4 4 4M16 8l4 4-4 4M13.5 5l-3 14',
  terminal: 'M4 5h16v14H4zM8 10l3 2-3 2M13 15h3',
  token: 'M13 15.5a5.5 5.5 0 1 1-11 0 5.5 5.5 0 0 1 11 0zM11.4 11.6 21 2M15.5 7.5l2.3 2.3a1 1 0 0 0 1.4 0l2.1-2.1a1 1 0 0 0 0-1.4L19 4',
  password: 'M3 8h18v8H3zM8 12h.01M12 12h.01M16 12h.01',
  lock: 'M6 11h12v9H6zM9 11V8a3 3 0 0 1 6 0v3',
  link: 'M10 13a5 5 0 0 0 7.5.5l3-3a5 5 0 0 0-7-7l-1.7 1.7M14 11a5 5 0 0 0-7.5-.5l-3 3a5 5 0 0 0 7 7l1.7-1.7',
  timezone: 'M12 3a9 9 0 1 0 0 18 9 9 0 0 0 0-18zM12 7v5l3 2',
  languages: 'M4 5h9M8.5 3v2M6 5c.8 3.2 2.8 5.6 5.5 7M11 5c-.9 3.6-3.3 6.4-7 8M13 20l4-9 4 9M14.3 17h5.4',
  app: 'M7 3h10v18H7zM11 17.5h2',
  passkey:
    'M12 10a2 2 0 0 0-2 2c0 1-.1 2.5-.26 4M14 13.1c0 2.4 0 6.4-1 8.9M17.3 21c.12-.6.43-2.3.5-3M2 12a10 10 0 0 1 18-6M2 16h.01M21.8 16c.2-2 .13-5.35 0-6M5 19.5C5.5 18 6 15 6 12a6 6 0 0 1 .34-2M8.65 22c.21-.66.45-1.32.57-2M9 6.8a6 6 0 0 1 9 5.2v2',
  recovery:
    'M12 2a10 10 0 1 0 0 20 10 10 0 0 0 0-20zM12 8a4 4 0 1 0 0 8 4 4 0 0 0 0-8zM4.9 4.9l4.3 4.3M14.8 9.2l4.3-4.3M14.8 14.8l4.3 4.3M9.2 14.8l-4.3 4.3',
  warning: 'M12 4l9 15H3zM12 10v4M12 17h.01',
  offline: 'M2 8.8a15 15 0 0 1 4.2-2.7M22 8.8A15 15 0 0 0 10.6 5M5 12.9a10 10 0 0 1 3.5-2.2M19 12.9a10 10 0 0 0-2-1.5M8.5 16.4a5 5 0 0 1 7 0M12 20h.01M2 2l20 20',
  slow: 'M12 21a9 9 0 1 0 0-18 9 9 0 0 0 0 18zM12 7v5l3 2',
} as const;
