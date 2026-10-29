// The viewer SDK: what a custom client builds its own interface on. `mountViewer` draws a session's screen into an
// element and sends the person's input; everything else (bars, menus, prompts) is the client's. Built to
// dist/sdk/viewer.js (ESM) with its types beside it; the admin serves both under /sdk/.
export { ALL_FEATURES } from './features.ts';
export { mountViewer, viewerToken } from './viewer.ts';
export type { Features } from './features.ts';
export type {
  Lease,
  RecordingRequest,
  RecordingState,
  Transfer,
  Viewer,
  ViewerOptions,
  ViewerStats,
} from './viewer.ts';
export type { SoundState } from './audio.ts';
export type { CopyOffer } from './copy.ts';
export type { Download } from './files.ts';
export type { BatchProgress, Upload } from './uploads.ts';
export { LeaseHolder, RecordPhase } from './protocol.ts';
// Recording with the person's camera and microphone: devices, capture, and where the camera sits.
// `wire` turns a layout into what `startRecording` and `recordLayout` take; `microphoneSupported` says whether this
// browser can send the microphone at all (a recording asked for one without it records none). The camera is a square
// whose side stays from `SIDE_MIN` to `SIDE_MAX` of the screen's width.
export { Capture, DEFAULT_LAYOUT, listDevices, microphoneSupported, openDevice, SIDE_MAX, SIDE_MIN, wire } from './recorder.ts';
export type { CameraLayout, Devices } from './recorder.ts';
