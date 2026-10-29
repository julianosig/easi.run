// What a session lets its viewers do, alone in its module: the design preview's mock (Node) takes it too.

/**
 * What a session lets its viewers do, as `GET /api/sessions` lists it (`features`); the admin enforces it whatever a
 * client sends. A recording is a download, so it needs `downloads` too.
 */
export interface Features {
  keyboard_pointer: boolean;
  clipboard: boolean;
  uploads: boolean;
  downloads: boolean;
  sound: boolean;
  recording: boolean;
}

/** Every feature on, as a session starts without `features`. */
export const ALL_FEATURES: Features = {
  keyboard_pointer: true,
  clipboard: true,
  uploads: true,
  downloads: true,
  sound: true,
  recording: true,
};
