<script lang="ts">
  // One session's screen. For a demo visitor (`demoLeft` set) it is the screen alone, full size, under a bar that hides
  // itself, and nothing else unless it matters (a file request, a refused copy, transfers); for the administrator it
  // adds the session's panels. The viewer is mounted for the life of this view (a demo's again when its stream drops) and
  // closed with it, timers included.
  import { untrack } from 'svelte';
  import { LeaseHolder } from '../protocol.ts';
  import { ALL_FEATURES, type Features, mountViewer, type Transfer as Batch, type Viewer, type ViewerStats, viewerToken } from '../viewer.ts';
  import Agent from './Agent.svelte';
  import Bar from './Bar.svelte';
  import DemoBar from './DemoBar.svelte';
  import RecordMenu from './RecordMenu.svelte';
  import { awaitAtMost, Capture, type CameraLayout, DEFAULT_LAYOUT, type Devices, listDevices, microphoneSupported, openDevice, parseLayout, wire } from '../recorder.ts';
  import CameraBubble from './CameraBubble.svelte';
  import type { SoundState } from '../audio.ts';
  import type { Download } from '../files.ts';
  import { RecordPhase } from '../protocol.ts';
  import type { RecordingState } from '../viewer.ts';
  import Encoder from './Encoder.svelte';
  import TransferCard from './TransferCard.svelte';
  import Volume from './Volume.svelte';
  import { api, currentVisit, formatSize, type Inspection, logout, messageOf, remember, remembered, type SessionInfo, statusOf, type Visit } from './api.ts';
  import { demoOf } from './demos.ts';
  import { announce, embedder, picks } from './embed.ts';
  import { button, card, CODEC_NAMES, panelTitle, primary } from './ui.ts';

  type Exit = 'sessions' | 'login' | 'end' | 'lost' | 'left' | 'moved';
  /** The close code of a stream whose demo was closed for one opened in another browser (admin `MOVED_CODE`). */
  const MOVED_CODE = 4001;
  /** How many times in a row a demo's dropped stream connects again before the visit counts as lost. */
  const MAX_RETRIES = 5;
  /** How long a stream must stay up before a later drop gets every retry anew (a recurring error must not loop). */
  const STEADY_MS = 10_000;
  let {
    id,
    demoLeft,
    demoSite = '',
    demoSites = [],
    demoSeconds = 60,
    demoEmbedders = [],
    onExit,
    onSwitch,
  }: {
    id: string;
    demoLeft?: number;
    demoSite?: string;
    demoSites?: string[];
    demoSeconds?: number;
    /** The pages that may frame the demo (see `embed.ts`). */
    demoEmbedders?: string[];
    onExit: (to: Exit) => void;
    /** The visitor's turn goes on in another session (they switched demos, here or in another tab). */
    onSwitch?: (visit: Visit) => void;
  } = $props();

  const demo = untrack(() => demoLeft !== undefined);
  // The page framing the demo, when one allowed to does: it lists the demos and picks one.
  const framer = demo ? embedder(untrack(() => demoEmbedders)) : null;
  const token = viewerToken();
  const ends = Date.now() + untrack(() => demoLeft ?? 0) * 1000;
  const saved = Number(remembered('volume') ?? 100);

  type Transfer = { name: string; detail: string };
  let screen: HTMLDivElement | undefined = $state();
  let holder: LeaseHolder | null = $state(null);
  let error = $state('');
  let copyOffered = $state(false);
  let copied = $state(false);
  let autoCopy = $state(remembered('autoCopy') === '1');
  let fileRequest: { choose: () => void; cancel: () => void } | null = $state(null);
  let uploads: Transfer[] = $state([]);
  let downloads: Transfer[] = $state([]);
  // Batches of files on their way to the page; the first one still going is drawn over the screen, a finished one for
  // a moment longer.
  let batches: Batch[] = $state([]);
  // Recording (the demo bar's, or beside the administrator's controls): the devices picked, the camera's preview, and the recording under way.
  let recordOpen = $state(false);
  let devices: Devices = $state({ microphones: [], cameras: [] });
  let micId = $state('');
  let camId = $state('');
  // Where the camera is and how it looks, remembered across visits.
  let cameraLayout: CameraLayout = $state(parseLayout(remembered('cameraLayout')) ?? DEFAULT_LAYOUT);
  // While dragging, a recording hears the camera's place at most every LAYOUT_EVERY_MS (the last one always).
  const LAYOUT_EVERY_MS = 100;
  let layoutSent = 0;
  let layoutLater: ReturnType<typeof setTimeout> | undefined;
  let pageSound = $state(true);
  let camStream: MediaStream | null = $state.raw(null);
  // The recording under way: the display records it; this view sends the camera and microphone (which it opened) until
  // the display says it ended.
  type Active = {
    id: number;
    phase: 'starting' | 'recording' | 'stopping';
    mic: MediaStream | null;
    capture: Capture | null;
    /** Its download, once the display offered it. */
    file: number | null;
    /** The display finished it: it ends once its download has every byte. */
    written: boolean;
    ended: () => void;
    done: Promise<void>;
  };
  let active: Active | null = $state.raw(null);
  let lastDownloads: Download[] = [];
  let recElapsed = $state(0);
  let recBusy = $state(false);
  let recNotice = $state('');
  let recClock: ReturnType<typeof setInterval> | undefined;
  // Said a while after a recording is saved, unless another recording's notice replaced it.
  const noteRecording = passing((text) => (recNotice = text), 8000);
  /** A recording stops this long before a demo visit ends, so its download is whole before the visit goes. */
  const RECORDING_MARGIN_MS = 10_000;
  let screenWidth = $state(0);
  let screenHeight = $state(0);
  const batchShown = $derived(batches.find((b) => b.state === 'sending' || b.state === 'finishing') ?? batches.at(-1));
  const batchTimers = new Set<ReturnType<typeof setTimeout>>();
  /** Drops every batch's progress, what was said of them, and their pending removals. */
  function forgetBatches(): void {
    for (const timer of batchTimers) clearTimeout(timer);
    batchTimers.clear();
    batches = [];
    transferSaid = [];
  }
  let transferSaid: { batch: number; text: string }[] = $state([]);
  let sound: SoundState = $state('playing');
  // Muted here, so a connection made again stays muted.
  let muted = false;
  const mute = (m: boolean) => {
    muted = m;
    viewer?.setMuted(m);
  };
  let autoFailed = $state(false);
  // View only: the visitor keeps the lease (so nobody else takes it) but sends no input. Remembered across demos.
  let viewOnly = $state(remembered('viewOnly') === '1');
  let switching: string | null = $state(null);
  let level = $state(Number.isFinite(saved) ? Math.min(100, Math.max(0, saved)) : 100);
  let stats: ViewerStats | null = $state(null);
  // The administrator's stream ended for good (a demo's connects again instead).
  let disconnected = $state(false);
  let containers: Inspection | undefined = $state();
  let left = $state(untrack(() => demoLeft ?? 0));

  let viewer: Viewer | null = null;
  // What the session lets its viewers do: every feature in the demo; otherwise as listed, before the viewer mounts.
  let features: Features | null = $state(demo ? ALL_FEATURES : null);
  // The session as listed (the administrator's): its encoder and network panels start from what it has now.
  let info: SessionInfo | null = $state(null);
  if (!demo) {
    api<{ sessions: SessionInfo[] }>('GET', '/api/sessions').then(
      (list) => {
        info = list.sessions.find((s) => s.id === id) ?? null;
        features = info?.features ?? ALL_FEATURES;
      },
      (e: unknown) => {
        if (gone) return;
        if (statusOf(e) === 401) return onExit('login');
        // Unlisted, the screen still mounts with every feature offered: the admin enforces the session's own whatever
        // this page sends, so the worst is a control the session then refuses (better than "Connecting…" for good).
        fail(e);
        features = ALL_FEATURES;
      },
    );
  }
  // Set when this view goes away: a late response must not navigate from it. Its own effect, so it is set even when the
  // screen never mounted (the feature lookup still out, say).
  let gone = false;
  $effect(() => () => (gone = true));
  const mine = $derived(holder === LeaseHolder.You);
  /** Shows each text through `set`, then clears it after `ms` unless a later one replaced it. */
  function passing(set: (text: string) => void, ms: number): (text: string) => void {
    let shown = 0;
    return (text) => {
      set(text);
      const turn = ++shown;
      setTimeout(() => {
        if (shown === turn) set('');
      }, ms);
    };
  }
  // What went wrong, in the banner. A demo's banner would otherwise cover the screen's foot for the rest of the visit, so
  // there every message goes after a few seconds, unless something else was said meanwhile; the administrator's stays.
  const sayAWhile = passing((text) => (error = text), 5000);
  function say(text: string): void {
    if (demo) sayAWhile(text);
    else error = text;
  }
  const fail = (e: unknown) => say(messageOf(e));

  $effect(() => {
    const target = screen;
    const allowed = features;
    if (!target || !allowed) return;
    // Mounted once per screen: nothing read while it is set up (a prop a switch in place updates, say) remounts it.
    return untrack(() => {
      let done = false;
      const leave = (to: Exit) => {
        if (done) return;
        done = true;
        onExit(to);
      };
      // The lease epoch of the last automatic takeover tried, and whether the next lease is this view's first (the tab
      // opened last takes control from the visitor's other tab, once); both outlive a connection tried again.
      let autoEpoch = -1;
      let firstLease = true;
      // Connections tried again since the stream last stayed up STEADY_MS (see `recover`), and when the current one's
      // first lease came (null before it).
      let retries = 0;
      let upSince: number | null = null;
      const connect = () => {
        upSince = null;
        viewer = mountViewer(target, {
          url: `${location.protocol === 'https:' ? 'wss' : 'ws'}://${location.host}/ws/sessions/${id}?viewer=${token}`,
          session: id,
          features: allowed,
          onLease: (lease) => {
            holder = lease.holder;
            upSince ??= Date.now();
            viewer?.setControl(lease.holder === LeaseHolder.You && !(demo && viewOnly));
            // A demo visitor is always in control: whenever the agent holds it, take it, once per lease; and the tab
            // opened last takes it from the visitor's other tab, once (that tab can take it back).
            // Only on this view's first lease: a tab here first, or connected again, never takes it back on its own.
            const fromOther = lease.holder === LeaseHolder.Other && firstLease;
            firstLease = false;
            // Control went elsewhere: a recording ended with it (the display says nothing more to this tab).
            if (lease.holder !== LeaseHolder.You && active) recordingEnded(false);
            if (demo && (lease.holder === LeaseHolder.Agent || fromOther) && lease.leaseEpoch !== autoEpoch) {
              const epoch = lease.leaseEpoch;
              const from = lease.holder;
              autoEpoch = epoch;
              autoFailed = false;
              api('POST', `/api/sessions/${id}/control`, { viewer: token, if_free: !fromOther }).catch((e: unknown) => {
                // A 404: the session is being replaced (a switch) or stopped; the stream's close says what comes next.
                if (done || autoEpoch !== epoch || holder !== from || statusOf(e) === 404) return;
                autoFailed = true;
                fail(e);
              });
            }
          },
          onCopied: (offer) => {
            copyOffered = offer !== null && !offer.written;
            copied = offer?.written ?? false;
          },
          onFileRequest: (request) => (fileRequest = request),
          onTransfer: (t) => {
            if (done) return;
            batches = [...batches.filter((b) => b.batch !== t.batch), t].sort((a, b) => a.batch - b.batch);
            if (t.state === 'done' || t.state === 'failed') {
              const what = t.files === 1 ? t.name : `${t.files} files`;
              const said = t.state === 'done' ? `${what} uploaded` : `The upload of ${what} failed`;
              // Each result is a new entry of a log, so the same result twice is said twice.
              transferSaid = [...transferSaid.slice(-2), { batch: t.batch, text: said }];
              // A failure stays said even when another batch's card covers it.
              if (t.state === 'failed') say(said);
              const timer = setTimeout(() => {
                batchTimers.delete(timer);
                batches = batches.filter((b) => b.batch !== t.batch);
              }, t.state === 'done' ? 1200 : 4000);
              batchTimers.add(timer);
            }
          },
          onUploads: (list) => {
            uploads = list.map((u) => ({
              name: u.name,
              detail: `${formatSize(u.size)} · ${u.state === 'stored' ? 'given to the page' : u.state === 'failed' ? 'failed' : `${Math.floor((100 * u.sent) / Math.max(u.size, 1))}%`}`,
            }));
          },
          onDownloads: (list) => {
            downloadsChanged(list);
            downloads = list.map((d) => ({
              name: d.name,
              detail: `${d.size === null ? '' : `${formatSize(d.size)} · `}${{ starting: 'starting', downloading: 'downloading (see your browser’s downloads)', saved: 'complete', failed: 'failed' }[d.state]}`,
            }));
          },
          onSound: (state) => (sound = state),
          onRecording: (state) => recordingChanged(state),
          onStats: (s) => (stats = s),
          onError: (e, fatal) => {
            // A dead stream holds nothing (no "You hold control", no Give back) and measures nothing, and a recording
            // went with it: the connection carried its camera, its microphone and its download.
            let lostRecording = false;
            if (fatal) {
              holder = null;
              stats = null;
              lostRecording = active !== null;
              if (lostRecording) recordingEnded(false);
            }
            // The visitor opened the demo in another browser: this one was closed for it.
            if (fatal && demo && (e as { closeCode?: number }).closeCode === MOVED_CODE) return leave('moved');
            if (fatal && demo) return void recover();
            if (fatal) disconnected = true;
            say(fatal ? `Stream ended: ${e.message}${lostRecording ? ' The recording stopped: nothing was saved.' : ''}` : e.message);
          },
        });
        viewer.setVolume(untrack(() => level) / 100);
        if (muted) viewer.setMuted(true);
        // What the page copies goes straight to the person's clipboard where the browser allows it (a demo visitor
        // always; the administrator by choice); otherwise a button offers it.
        viewer.setAutoCopy(demo || untrack(() => autoCopy));
      };
      connect();
    // The stream ended: the connection dropped while the visit goes on, or the turn is over. On the same session (its
    // page switched in place, or the network blinked) the screen connects again, a few times with growing pauses; a
    // visit gone to another session follows it.
    async function recover() {
      if (upSince !== null && Date.now() - upSince >= STEADY_MS) retries = 0;
      while (!done && Date.now() < ends - 1000) {
        try {
          const visit = await currentVisit();
          if (done) return;
          if (!visit) break;
          if (visit.session !== id) {
            done = true;
            return onSwitch?.(visit);
          }
          if (retries >= MAX_RETRIES) break;
          await new Promise((r) => setTimeout(r, Math.min(500 * 2 ** retries++, 4000)));
          if (done) return;
          holder = null;
          viewer?.close();
          // The new connection numbers its batches from one again: the old ones' progress and timers go first.
          forgetBatches();
          return connect();
        } catch (e) {
          if (statusOf(e) === 401) break;
        }
        await new Promise((r) => setTimeout(r, 1000));
      }
      leave(Date.now() >= ends - 1000 ? 'end' : 'lost');
    }
    // The administrator's panel hears the containers; a demo tab only which demo its visit shows (another tab of the
    // visit switched: this one follows), from /api/me, which costs the admin no Docker inspection per visitor. The next
    // poll waits for the previous one to settle (a Docker inspection can take longer than the interval), so answers
    // never pile up or land out of order.
    let poll: ReturnType<typeof setTimeout> | undefined;
    const check = async (): Promise<void> => {
      try {
        if (demo) {
          const visit = await currentVisit();
          if (!done && visit && visit.site !== demoSite && switching === null) onSwitch?.(visit);
          return;
        }
        const data = await api<{ containers?: Inspection }>('GET', `/api/sessions/${id}/stats`);
        if (!done) containers = data.containers;
      } catch (e) {
        // A demo's login lapsed before its time ran out (the visitor left in another window, say): lost, not over, so
        // the end page starts no new visit on its own.
        if (statusOf(e) === 401) leave(!demo ? 'login' : Date.now() >= ends - 1000 ? 'end' : 'lost');
      } finally {
        if (!done) poll = setTimeout(check, 2000);
      }
    };
    poll = setTimeout(check, 2000);
    const clock = demo
      ? setInterval(() => {
          left = Math.max(0, Math.ceil((ends - Date.now()) / 1000));
          if (left === 0) leave('end');
          if (active && active.phase !== 'stopping' && ends - Date.now() < RECORDING_MARGIN_MS) void finishRecording();
        }, 250)
      : undefined;
    return () => {
      done = true;
      forgetBatches();
      clearInterval(recClock);
      // Leaving and switching wait for a recording to end; anything else that ends the view ends it too.
      void finishRecording();
      active?.capture?.stop();
      stopTracks(active?.mic ?? null);
      stopTracks(camStream);
      camStream = null;
      clearTimeout(poll);
      clearInterval(clock);
      viewer?.close();
      viewer = null;
    };
    });
  });

  // A file or link dropped beside the screen does nothing: the browser would otherwise open it in place of this page,
  // and the visit with it. The screen's own handlers have already decided the drags over it.
  $effect(() => {
    const refuse = (e: DragEvent) => {
      const types = e.dataTransfer?.types ?? [];
      if (e.defaultPrevented || !(types.includes('Files') || types.includes('text/uri-list'))) return;
      e.preventDefault();
      if (e.dataTransfer) e.dataTransfer.dropEffect = 'none';
    };
    addEventListener('dragover', refuse);
    addEventListener('drop', refuse);
    return () => {
      removeEventListener('dragover', refuse);
      removeEventListener('drop', refuse);
    };
  });

  const take = () => api('POST', `/api/sessions/${id}/control`, { viewer: token }).catch(fail);
  const giveBack = () => api('POST', `/api/sessions/${id}/release`, { viewer: token }).catch(fail);
  const copy = () => viewer?.writeCopied().catch(fail);
  function setViewOnly(on: boolean) {
    viewOnly = on;
    remember('viewOnly', on ? '1' : '0');
    viewer?.setControl(mine && !on);
  }
  // A framing page hears the demos and the one on, and picks one.
  $effect(() => {
    if (!framer) return;
    announce(framer, demoSites, switching ?? demoSite, true);
    // The view going (the visit ended, or a switch replaces it) says so; the next view says it is back.
    return () => announce(framer, demoSites, untrack(() => demoSite), false);
  });
  $effect(() => {
    if (!framer) return;
    return picks(
      framer,
      () => demoSites,
      (name) => {
        if (name !== (switching ?? demoSite)) void switchTo(name);
      },
    );
  });
  // A switch opens the demo's page in this same browser (the visit, its time and a recording under way go on). Picks
  // made while one is on its way wait, and only the last of them goes.
  let nextPick: string | null = null;
  async function switchTo(name: string) {
    if (switching !== null) {
      nextPick = name;
      switching = name;
      return;
    }
    switching = name;
    error = '';
    try {
      let visit: Visit | null = null;
      for (let pick: string | null = name; pick !== null; pick = nextPick) {
        nextPick = null;
        const r = await api<{ demo: Visit | null }>('POST', '/api/demo/switch', { demo: pick, viewer: token });
        if (gone) return;
        visit = r.demo;
        if (!visit) break;
      }
      switching = null;
      if (visit) return onSwitch?.(visit);
      onExit('end');
    } catch (e) {
      if (gone) return;
      switching = null;
      nextPick = null;
      if (statusOf(e) === 410) return onExit('end');
      fail(e);
    }
  }
  function setAutoCopy(on: boolean) {
    autoCopy = on;
    remember('autoCopy', on ? '1' : '0');
    viewer?.setAutoCopy(on);
  }
  const stopTracks = (stream: MediaStream | null) => stream?.getTracks().forEach((t) => t.stop());

  async function refreshDevices() {
    devices = await listDevices().catch(() => devices);
  }
  // The camera is open while the menu shows it, or while recording; closed otherwise.
  $effect(() => {
    const wanted = camId !== '' && (recordOpen || active !== null);
    if (!wanted) {
      if (camStream) {
        stopTracks(camStream);
        camStream = null;
      }
      return;
    }
    if (camStream) return;
    const id = camId;
    const pick = camPicks;
    openDevice('camera', id).then(
      (stream) => {
        // Changed or closed meanwhile: this one is not needed.
        if (gone || pick !== camPicks || camStream) return stopTracks(stream);
        camStream = stream;
        void refreshDevices();
      },
      (e: unknown) => {
        // A camera picked since then (even the same one again) is the one that counts: this failure says nothing of it.
        if (gone || pick !== camPicks) return;
        camId = '';
        say(`The camera could not open: ${messageOf(e)}`);
      },
    );
  });
  // Each pick of a device counts up, so a late answer to an earlier pick (of the same device, too) is told apart.
  let camPicks = 0;
  let micPicks = 0;
  function pickCamera(id: string) {
    stopTracks(camStream);
    camStream = null;
    camPicks += 1;
    camId = id;
  }
  async function pickMicrophone(id: string) {
    micId = id;
    const pick = ++micPicks;
    if (id === '') return;
    // Asked once here, so the browser asks now and the names show; opened again when recording starts.
    try {
      stopTracks(await openDevice('microphone', id));
      void refreshDevices();
    } catch (e) {
      // A microphone picked since then (even the same one again) is the one that counts: this failure says nothing of
      // it.
      if (gone || pick !== micPicks) return;
      micId = '';
      say(`The microphone could not open: ${messageOf(e)}`);
    }
  }
  async function startRecording() {
    if (!viewer || active) return;
    recBusy = true;
    let mic: MediaStream | null = null;
    const chosen = { micId, camId };
    try {
      // A browser that cannot send the microphone neither opens it nor tells the display of one.
      mic = micId && microphoneSupported() ? await openDevice('microphone', micId) : null;
      // Changed while the microphone opened (the menu is locked meanwhile, but the view may have gone): start again.
      if (gone || !viewer || micId !== chosen.micId || camId !== chosen.camId || (camId !== '' && camStream === null)) {
        return stopTracks(mic);
      }
      const id = viewer.startRecording({ layout: wire(cameraLayout), camera: camStream !== null, microphone: mic !== null, pageSound });
      if (id === null) {
        stopTracks(mic);
        say('Recording needs your turn at the screen.');
        return;
      }
      let ended = () => {};
      const done = new Promise<void>((resolve) => (ended = resolve));
      const started = Date.now();
      active = { id, phase: 'starting', mic, capture: null, file: null, written: false, ended, done };
      recNotice = '';
      recElapsed = 0;
      recClock = setInterval(() => (recElapsed = Math.floor((Date.now() - started) / 1000)), 500);
      recordOpen = false;
    } catch (e) {
      stopTracks(mic);
      say(`The recording could not start: ${messageOf(e)}`);
    } finally {
      recBusy = false;
    }
  }
  /** The display said where the recording is: it started (the camera and microphone go to it from now), or it ended. */
  function recordingChanged(state: RecordingState) {
    const a = active;
    if (!a || state.recording !== a.id) return;
    if (state.phase === RecordPhase.Recording) {
      if (a.phase === 'starting' && viewer) {
        active = { ...a, phase: 'recording', file: state.file, capture: new Capture(viewer, a.id, camStream, a.mic) };
        // The camera may have moved while the recording started.
        sendLayout();
      }
      return;
    }
    if (state.phase === RecordPhase.Failed) return recordingEnded(false);
    // Written whole: it ends when its download has the last byte (Leave and switching wait for that).
    a.capture?.stop();
    active = { ...a, phase: 'stopping', capture: null, file: state.file, written: true };
    downloadsChanged(lastDownloads);
  }
  /** The recording's download finished (or failed) after the display wrote it: the recording ends. */
  function downloadsChanged(list: Download[]) {
    lastDownloads = list;
    const a = active;
    if (!a?.written) return;
    const d = list.find((x) => x.file === a.file);
    if (d?.state === 'saved') recordingEnded(true);
    else if (d === undefined || d.state === 'failed') recordingEnded(false);
  }
  function recordingEnded(saved: boolean) {
    const a = active;
    if (!a) return;
    a.capture?.stop();
    stopTracks(a.mic);
    clearInterval(recClock);
    active = null;
    a.ended();
    if (!saved) say('The recording stopped: nothing was saved.');
    else noteRecording('Recording saved to your Downloads');
  }
  /**
   * Stops the recording; resolves once its download is whole (or failed, or after a while without an answer). A display
   * that never says the file is written by then ends the recording as failed, so it cannot stay "Saving…" for good; a
   * download already under way goes on.
   */
  function finishRecording(): Promise<void> {
    const a = active;
    if (!a) return Promise.resolve();
    if (a.phase !== 'stopping') {
      // Stopping before the display hears it: a stream that fails as it is told ends the recording, for good.
      active = { ...a, phase: 'stopping', capture: null };
      a.capture?.stop();
      // Nothing more goes to the display: the microphone closes now (the camera once the recording ends).
      stopTracks(a.mic);
      viewer?.stopRecording(a.id);
    }
    // Late, the recording is still this one: only its end settles `done`, and none starts before it ended.
    return awaitAtMost(a.done, 15_000, () => {
      if (active && !active.written) recordingEnded(false);
    });
  }
  /** The camera moved or changed its look: the preview follows at once, a recording soon after. */
  function layoutChanged(next: CameraLayout, settled: boolean) {
    cameraLayout = next;
    if (settled) remember('cameraLayout', JSON.stringify(next));
    clearTimeout(layoutLater);
    const wait = LAYOUT_EVERY_MS - (performance.now() - layoutSent);
    if (settled || wait <= 0) sendLayout();
    else layoutLater = setTimeout(sendLayout, wait);
  }
  function sendLayout() {
    const a = active;
    if (!viewer || a?.phase !== 'recording') return;
    layoutSent = performance.now();
    viewer.recordLayout(a.id, wire(cameraLayout));
  }

  async function signOut() {
    // A recording ends whole before the stream goes.
    await finishRecording();
    // A demo visitor who leaves sees that they left (reloading would start a new visit), even if the stream closes
    // before the logout is answered.
    if (demo) {
      onExit('left');
      void logout();
      return;
    }
    await logout();
    location.reload();
  }
  /** Lets a recording under way end whole before something outside this view takes it away (the browser's Back). */
  export function leave(): Promise<void> {
    return finishRecording();
  }
  /** Back to the sessions, once a recording under way is whole. */
  async function toSessions() {
    await finishRecording();
    if (!gone) onExit('sessions');
  }
  async function close() {
    // The recording's file comes from the session: it ends whole before the session goes.
    await finishRecording();
    // A failed close stays here, where its error shows (the Sessions list would hide it).
    try {
      await api('DELETE', `/api/sessions/${id}`);
    } catch (e) {
      return fail(e);
    }
    if (!gone) onExit('sessions');
  }

  const leaseText = $derived(
    disconnected
      ? 'Disconnected'
      : holder === null
        ? 'Connecting…'
        : mine
          ? 'You hold control'
          : holder === LeaseHolder.Other
            ? 'Another viewer holds control'
            : 'Watching (the agent may act)',
  );
  const statRows = $derived.by((): [string, string][] => {
    const s = stats;
    if (!s) return [];
    return [
      ['Presented', `${s.presented} fps`],
      ['Codec', s.codec === null ? '—' : CODEC_NAMES[s.codec]],
      ['Hardware decoding', s.hardware === null ? 'unknown' : s.hardware ? 'yes' : 'no'],
      ['Drawing', s.renderer === 'webgl2' ? 'WebGL2' : 'Canvas 2D (no hardware WebGL2)'],
      ['Media', `${((s.mediaBytes * 8) / 1000).toFixed(0)} kbit/s`],
      ['Decode p50', `${(s.decodeUsP50 / 1000).toFixed(1)} ms`],
      ['Composition to draw p50', s.composeToDrawUsP50 === null ? '—' : `${(s.composeToDrawUsP50 / 1000).toFixed(0)} ms`],
      ['Encode p50 / p95', s.server ? `${(s.server.encodeUsP50 / 1000).toFixed(1)} / ${(s.server.encodeUsP95 / 1000).toFixed(1)} ms` : '—'],
      ['Encoded', s.server ? `${s.server.frames} fps` : '—'],
      ['Clock offset', s.clockOffsetUs === null ? '—' : `±${((s.clockErrorUs ?? 0) / 1000).toFixed(1)} ms`],
    ];
  });
  const containerRows = $derived.by((): [string, string][] => {
    const byRole = containers?.containers ?? {};
    const roles = Object.keys(byRole).sort();
    if (roles.length === 0) return [['State', containers?.error ?? '—']];
    return roles.map((role) => {
      const c = byRole[role]!;
      const cpu = c.cpu_percent != null ? `${Number(c.cpu_percent).toFixed(0)} % CPU` : '—';
      const memory = c.memory_bytes != null ? `${(Number(c.memory_bytes) / 1048576).toFixed(0)} MiB` : '—';
      return [role, `${c.status ?? ''} · ${cpu} · ${memory}`];
    });
  });
  const residual =
    'While you hold control, what you type or paste (four characters or more) is removed from the agent’s later observations.';
</script>

{#snippet volume()}
  <Volume {sound} bind:level onMute={mute} onVolume={(v) => viewer?.setVolume(v)} />
{/snippet}

{#snippet transfers(title: string, list: Transfer[])}
  {#if list.length > 0}
    <ul class="flex flex-wrap gap-2" aria-label={title}>
      {#each list as t, i (i)}<li class="rounded-full border border-line bg-paper px-3 py-1.5 text-sm"><span class="font-semibold">{t.name}</span> <span class="text-muted">{t.detail}</span></li>{/each}
    </ul>
  {/if}
{/snippet}

{#snippet recorder()}
  <RecordMenu
    bind:open={recordOpen}
    {devices}
    {micId}
    {camId}
    bind:pageSound
    layout={cameraLayout}
    onLayout={(next) => layoutChanged(next, true)}
    recording={active !== null}
    stopping={active?.phase === 'stopping'}
    elapsed={recElapsed}
    busy={recBusy || (camId !== '' && camStream === null)}
    onMic={pickMicrophone}
    onCamera={pickCamera}
    onStart={startRecording}
    onStop={() => void finishRecording()}
  />
{/snippet}

{#snippet camera()}
  {#if camStream && screenWidth > 0}
    <!-- The camera where the recording puts it, live: what you see is what is recorded. -->
    <CameraBubble stream={camStream} layout={cameraLayout} width={screenWidth} height={screenHeight} onLayout={layoutChanged} />
  {/if}
{/snippet}

{#snippet extras()}
  {#if fileRequest && !(demo && viewOnly)}
    <div class="flex flex-wrap items-center gap-2">
      <button class={primary} type="button" onclick={() => fileRequest?.choose()}>Choose files for the page</button>
      <button class={button} type="button" onclick={() => fileRequest?.cancel()}>Cancel</button>
    </div>
  {/if}
  {#if copyOffered}<button class={button} type="button" onclick={copy}>Copy what the page copied</button>{/if}
  <span class="text-sm font-semibold text-muted empty:hidden {demo ? 'sr-only' : ''}" role="status">{copied ? 'Copied to your clipboard' : ''}</span>
  <span class="sr-only" role="status">{fileRequest ? 'The page asks for files' : ''}</span>
  <div class="sr-only" role="log" aria-live="polite">{#each transferSaid as entry (entry.batch)}<p>{entry.text}</p>{/each}</div>
  {#if !demo}{@render transfers('Uploads', uploads)}{/if}
  {@render transfers('Downloads', downloads)}
{/snippet}

{#if demo}
  <DemoBar
    sites={demoSites}
    site={demoSite}
    {switching}
    onPick={switchTo}
    picker={framer === null}
    {holder}
    {autoFailed}
    {viewOnly}
    onViewOnly={setViewOnly}
    onTake={take}
    {stats}
    {sound}
    bind:level
    onMute={mute}
    onVolume={(v) => viewer?.setVolume(v)}
    {left}
    total={demoSeconds}
    onLeave={signOut}
    pinned={recordOpen || active !== null}
    record={recorder}
  />
  <!-- Framed by a page, the screen fills the frame: that page draws the frame's edge, only the control ring shows. -->
  <main class="fixed inset-0 grid place-items-center bg-wash {framer ? 'p-[3px]' : 'p-2'}">
    <div class="relative" bind:clientWidth={screenWidth} bind:clientHeight={screenHeight}>
      <div
        class="relative overflow-hidden rounded-xl bg-[#0f1b33] transition {mine ? (viewOnly ? 'ring-2 ring-bad' : 'ring-2 ring-accent') : 'ring-1 ring-line'} {framer
          ? '[&_canvas]:max-h-[calc(100vh-6px)]! [&_canvas]:max-w-[calc(100vw-6px)]!'
          : 'shadow-[0_30px_80px_-40px_rgb(20_40_75/0.55)] [&_canvas]:max-h-[calc(100vh-1rem)]! [&_canvas]:max-w-[calc(100vw-1rem)]!'}"
        bind:this={screen}
      ></div>
      {#if batchShown}<TransferCard transfer={batchShown} width={screenWidth} height={screenHeight} />{/if}
      {@render camera()}
    </div>
    {#if switching !== null}
      <div class="absolute inset-0 grid place-items-center bg-wash/80 backdrop-blur-sm" role="status">
        <p class="flex items-center gap-3 rounded-2xl bg-paper px-5 py-3.5 font-semibold shadow-[0_24px_60px_-24px_rgb(20_40_75/0.45)]">
          <span class="size-4 animate-spin rounded-full border-2 border-accent border-t-transparent" aria-hidden="true"></span>
          Opening {demoOf(switching).title}…
        </p>
      </div>
    {/if}
  </main>
  <div class="pointer-events-none fixed inset-x-0 bottom-4 z-20 flex flex-col items-center gap-2 px-4 [&>*]:pointer-events-auto">
    <div class="flex flex-wrap items-center justify-center gap-3 empty:hidden">{@render extras()}</div>
    {#if recNotice}<p class="rounded-full bg-paper px-4 py-2 text-sm font-semibold shadow-[0_18px_44px_-14px_rgb(20_40_75/0.45)] ring-1 ring-line" role="status">{recNotice}</p>{/if}

    {#if error}<p class="rounded-full bg-bad px-4 py-2 text-sm font-semibold text-white" role="alert">{error}</p>{/if}
  </div>
{:else}
  <div class="px-4 pt-3">
    <Bar>
      <button class={button} type="button" onclick={toSessions}>Sessions</button>
      <button class={button} type="button" onclick={close}>Close session</button>
      <button class={button} type="button" onclick={signOut}>Log out</button>
    </Bar>
  </div>
  <main class="mx-auto grid max-w-[76rem] gap-4 px-4 pb-10 pt-5 lg:grid-cols-[minmax(0,1fr)_21rem]">
    <section class="min-w-0">
      <div class="flex flex-wrap items-center gap-2.5">
        <button class={primary} type="button" onclick={take} disabled={mine || holder === null}>Take control</button>
        <button class={button} type="button" onclick={giveBack} disabled={!mine}>Give back</button>
        <span class="text-muted" aria-live="polite">{leaseText}</span>
        <span class="flex-1"></span>
        {#if features?.sound}{@render volume()}{/if}
        {#if features?.recording}{@render recorder()}{/if}
      </div>
      {#if features?.clipboard}
        <label class="mt-3 flex items-center gap-2 text-sm">
          <input class="accent-accent" type="checkbox" checked={autoCopy} onchange={(e) => setAutoCopy(e.currentTarget.checked)} /> Copy what the page copies automatically
        </label>
      {/if}
      <div class="mt-3 flex flex-wrap items-center gap-3">{@render extras()}</div>
      {#if mine}<p class="mt-3 text-sm text-muted">{residual}</p>{/if}
      {#if recNotice}<p class="mt-3 text-sm font-semibold" role="status">{recNotice}</p>{/if}
      <p class="mt-3 font-semibold text-bad empty:hidden" role="alert">{error}</p>
      <div class="relative mt-4 w-fit max-w-full" bind:clientWidth={screenWidth} bind:clientHeight={screenHeight}>
        <div class="overflow-hidden rounded-2xl bg-[#0f1b33] ring-1 ring-line" bind:this={screen}></div>
        {#if batchShown}<TransferCard transfer={batchShown} width={screenWidth} height={screenHeight} />{/if}
        {@render camera()}
      </div>
    </section>
    <aside class="space-y-4">
      <section class={card}>
        <h2 class={panelTitle}>Stream</h2>
        <dl class="grid grid-cols-[auto_1fr] gap-x-3 gap-y-0.5 text-sm">
          {#each statRows as [k, v] (k)}<dt class="text-muted">{k}</dt><dd class="m-0 tabular-nums">{v}</dd>{/each}
        </dl>
      </section>
      <section class={card}>
        <h2 class={panelTitle}>Containers</h2>
        <dl class="grid grid-cols-[auto_1fr] gap-x-3 gap-y-0.5 text-sm">
          {#each containerRows as [k, v] (k)}<dt class="text-muted">{k}</dt><dd class="m-0 tabular-nums">{v}</dd>{/each}
        </dl>
      </section>
      {#if info}<section class={card}><Encoder {id} {fail} settings={info.settings} network={info.network} /></section>{/if}
      <section class={card}><Agent {id} /></section>
    </aside>
  </main>
{/if}
