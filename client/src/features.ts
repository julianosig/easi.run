// What a session lets its viewers do, alone in its module: the design preview's mock (Node) takes it too.

/** Every feature on, as a session starts without `features`. */
export const ALL_FEATURES = {
  keyboard_pointer: true,
  clipboard: true,
  uploads: true,
  downloads: true,
  sound: true,
  recording: true,
};

/**
 * What a session lets its viewers do, as `GET /api/sessions` lists it (`features`); the admin enforces it whatever a
 * client sends. A recording is a download, so it needs `downloads` too.
 */
export type Features = typeof ALL_FEATURES;
