<script lang="ts">
  // One session's screen: the screen alone, full size, under a bar that hides itself (the demo visitor's, or the
  // administrator's), and nothing else unless it matters (a file request, a refused copy, transfers). The viewer is mounted for the life of this view (a demo's again when its stream drops) and
  // closed with it, timers included.
  import { ICONS } from './icons.ts';
  import Button from './Button.svelte';
  import Icon from './Icon.svelte';
  import { untrack } from 'svelte';
  import { i18n, type Said, t } from '../i18n/index.svelte.ts';
  import { LeaseHolder, RecordPhase } from '../protocol.ts';
  import { ALL_FEATURES, type Features } from '../features.ts';
  import { mountViewer, type RecordingState, type Transfer as Batch, type Viewer, type ViewerStats, viewerToken } from '../viewer.ts';
  import SessionBar from './SessionBar.svelte';
  import RecordMenu from './RecordMenu.svelte';
  import { awaitAtMost } from '../deadline.ts';
  import { Capture, type CameraLayout, DEFAULT_LAYOUT, type Devices, listDevices, microphoneSupported, openDevice, parseLayout, wire } from '../recorder.ts';
  import CameraBubble from './CameraBubble.svelte';
  import type { SoundState } from '../audio.ts';
  import type { Download } from '../files.ts';
  import TransferCard from './TransferCard.svelte';
  import { api, currentVisit, formatSize, logout, me, remember, remembered, type SessionInfo, statusOf, type Visit } from './api.ts';
  import { errorOf } from './errors.ts';
  import { demoOf } from './demos.ts';
  import { announce, embedder, picks } from './embed.ts';

  type Exit = 'sessions' | 'closed' | 'login' | 'end' | 'lost' | 'left' | 'moved';
  /** The close code of a stream whose demo was closed for one opened in another browser (admin `MOVED_CODE`). */
  const MOVED_CODE = 4001;
  /** How many times in a row a demo's dropped stream connects again before the visit counts as lost. */
  const MAX_RETRIES = 5;
  /** How long a stream must stay up before a later drop gets every retry anew (a recurring error must not loop). */
  const STEADY_MS = 10_000;
  /** How long the screen may take to show its first picture before the page says it is taking long. */
  const SLOW_MS = 20_000;
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

  type Transfer = { name: string; detail: Said };
  let screen: HTMLDivElement | undefined = $state();
  let holder: LeaseHolder | null = $state(null);
  let error: Said | null = $state(null);
  let copyOffered = $state(false);
  let copied = $state(false);
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
    /** The lease it was started under: it ends with it. */
    leaseEpoch: number;
    ended: () => void;
    done: Promise<void>;
  };
  let active: Active | null = $state.raw(null);
  let lastDownloads: Download[] = [];
  let recElapsed = $state(0);
  let recBusy = $state(false);
  let recNotice: Said | null = $state(null);
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
  let transferSaid: { batch: number; text: Said }[] = $state([]);
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
  /** Bumped as a switch starts and as it settles: a poll's answer asked before either says nothing of the demo now. */
  let switches = 0;
  let level = $state(Number.isFinite(saved) ? Math.min(100, Math.max(0, saved)) : 100);
  let stats: ViewerStats | null = $state(null);
  // The lease last announced (see `Active`).
  let leaseEpoch = 0;
  // The administrator's stream ended for good (a demo's connects again instead).
  let disconnected = $state(false);
  // The screen has a picture: until then the page shimmers where it will be, never an empty dark box (the screen stays
  // there, transparent, so its keyboard can take focus).
  let pictured = $state(false);
  // No picture came within SLOW_MS of a connection.
  let slow = $state(false);
  let slowTimer: ReturnType<typeof setTimeout> | undefined;
  /** What went wrong, said with what to do: the stream dropped, or the first picture is taking long. */
  const problem = $derived(disconnected ? 'lost' : slow && !pictured ? 'slow' : null);
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
  function passing(set: (text: Said | null) => void, ms: number): (text: Said) => void {
    let shown = 0;
    return (text) => {
      set(text);
      const turn = ++shown;
      setTimeout(() => {
        if (shown === turn) set(null);
      }, ms);
    };
  }
  // What went wrong, in the banner. A demo's banner would otherwise cover the screen's foot for the rest of the visit, so
  // there every message goes after a few seconds, unless something else was said meanwhile; the administrator's stays.
  const sayAWhile = passing((text) => (error = text), 5000);
  function say(text: Said): void {
    if (demo) sayAWhile(text);
    else error = text;
  }
  const fail = (e: unknown) => say(errorOf(e));
  /** What the browser said when a device or the recording failed (its own words, never the admin's). */
  const browserSays = (e: unknown) => (e as Error).message;

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
        pictured = false;
        slow = false;
        clearTimeout(slowTimer);
        slowTimer = setTimeout(() => (slow = true), SLOW_MS);
        viewer = mountViewer(target, {
          url: `${location.protocol === 'https:' ? 'wss' : 'ws'}://${location.host}/ws/sessions/${id}?viewer=${token}`,
          session: id,
          features: allowed,
          onLease: (lease) => {
            holder = lease.holder;
            leaseEpoch = lease.leaseEpoch;
            upSince ??= Date.now();
            viewer?.setControl(lease.holder === LeaseHolder.You && !(demo && viewOnly));
            // A demo visitor is always in control: whenever the agent holds it, take it, once per lease; and the tab
            // opened last takes it from the visitor's other tab, once (that tab can take it back).
            // Only on this view's first lease: a tab here first, or connected again, never takes it back on its own.
            const fromOther = lease.holder === LeaseHolder.Other && firstLease;
            firstLease = false;
            // Control went elsewhere, or was taken again (a new lease): a recording ended with its lease (the display
            // says nothing more to this tab).
            if (active && (lease.holder !== LeaseHolder.You || lease.leaseEpoch !== active.leaseEpoch)) recordingEnded(false);
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
          onTransfer: (batch) => {
            if (done) return;
            batches = [...batches.filter((b) => b.batch !== batch.batch), batch].sort((a, b) => a.batch - b.batch);
            if (batch.state === 'done' || batch.state === 'failed') {
              const { files, name } = batch;
              const said: Said =
                batch.state === 'done'
                  ? () => (files === 1 ? t('session.uploaded.file', { name }) : t('session.uploaded.files', { count: files }))
                  : () => (files === 1 ? t('session.uploadFailed.file', { name }) : t('session.uploadFailed.files', { count: files }));
              // Each result is a new entry of a log, so the same result twice is said twice.
              transferSaid = [...transferSaid.slice(-2), { batch: batch.batch, text: said }];
              // A failure stays said even when another batch's card covers it.
              if (batch.state === 'failed') say(said);
              const timer = setTimeout(() => {
                batchTimers.delete(timer);
                batches = batches.filter((b) => b.batch !== batch.batch);
              }, batch.state === 'done' ? 1200 : 4000);
              batchTimers.add(timer);
            }
          },
          onUploads: (list) => {
            uploads = list.map((u) => ({
              name: u.name,
              detail: () =>
                t('session.transfer.detail', {
                  size: formatSize(u.size, i18n.locale),
                  state:
                    u.state === 'stored'
                      ? t('session.upload.stored')
                      : u.state === 'failed'
                        ? t('session.upload.failed')
                        : t('session.percent', { percent: Math.floor((100 * u.sent) / Math.max(u.size, 1)) }),
                }),
            }));
          },
          onDownloads: (list) => {
            downloadsChanged(list);
            downloads = list.map((d) => ({
              name: d.name,
              detail: () => {
                const state = t(`session.download.${d.state}`);
                return d.size === null ? state : t('session.transfer.detail', { size: formatSize(d.size, i18n.locale), state });
              },
            }));
          },
          onSound: (state) => (sound = state),
          onRecording: (state) => recordingChanged(state),
          onStats: (s) => (stats = s),
          onPicture: () => {
            pictured = true;
            slow = false;
            clearTimeout(slowTimer);
          },
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
            // Said with what to do where the screen was (a recording it took is said lost already).
            if (fatal) return void (disconnected = true);
            const message = e.message;
            say(() => message);
          },
        });
        viewer.setVolume(untrack(() => level) / 100);
        if (muted) viewer.setMuted(true);
        // What the page copies goes straight to the person's clipboard where the browser allows it; otherwise a button
        // offers it.
        viewer.setAutoCopy(true);
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
    // Both hear from /api/me whether the login still holds; a demo tab also which demo its visit shows (another tab of
    // the visit switched: this one follows). The next poll waits for the previous one to settle, so answers never pile
    // up or land out of order.
    let poll: ReturnType<typeof setTimeout> | undefined;
    const check = async (): Promise<void> => {
      try {
        if (demo) {
          const asked = switches;
          const visit = await currentVisit();
          if (!done && visit && visit.site !== demoSite && switching === null && asked === switches) onSwitch?.(visit);
          return;
        }
        await me();
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
      clearTimeout(slowTimer);
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
  // A recording under way ends first, its file saved: giving the control back ends the recording's download.
  async function giveBack() {
    await finishRecording();
    if (!gone) await api('POST', `/api/sessions/${id}/release`, { viewer: token }).catch(fail);
  }
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
      () => announce(framer, demoSites, switching ?? demoSite, true),
    );
  });
  // A switch opens the demo's page in this same browser (the visit, its time and a recording under way go on). Picks
  // made while one is on its way wait, and only the last of them goes.
  let nextPick: string | null = null;
  async function switchTo(name: string) {
    switches += 1;
    if (switching !== null) {
      nextPick = name;
      switching = name;
      return;
    }
    switching = name;
    error = null;
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
      switches += 1;
      if (visit) return onSwitch?.(visit);
      onExit('end');
    } catch (e) {
      if (gone) return;
      switching = null;
      switches += 1;
      nextPick = null;
      if (statusOf(e) === 410) return onExit('end');
      fail(e);
    }
  }
  const stopTracks = (stream: MediaStream | null) => stream?.getTracks().forEach((t) => t.stop());

  // The menu lists the devices there are now, so a working second camera is there to pick when the default is not.
  $effect(() => {
    if (recordOpen) void refreshDevices();
  });
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
        const message = browserSays(e);
        say(() => t('session.camera.failed', { error: message }));
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
      const message = browserSays(e);
      say(() => t('session.microphone.failed', { error: message }));
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
        say(() => t('session.record.needsTurn'));
        return;
      }
      let ended = () => {};
      const done = new Promise<void>((resolve) => (ended = resolve));
      const started = Date.now();
      active = { id, phase: 'starting', mic, capture: null, file: null, written: false, leaseEpoch, ended, done };
      recNotice = null;
      recElapsed = 0;
      recClock = setInterval(() => (recElapsed = Math.floor((Date.now() - started) / 1000)), 500);
      recordOpen = false;
    } catch (e) {
      stopTracks(mic);
      const message = browserSays(e);
      say(() => t('session.record.startFailed', { error: message }));
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
    if (!saved) say(() => t('session.record.lost'));
    else noteRecording(() => t('session.record.saved'));
  }
  /**
   * Stops the recording; resolves once its download is whole (or failed). A display that does not say the file is
   * written within a while ends the recording as failed, so it cannot stay "Saving…" for good; once it is written, the
   * wait is its download's, however long it takes (closing the session meanwhile would cut it short).
   */
  async function finishRecording(): Promise<void> {
    const a = active;
    if (!a) return;
    if (a.phase !== 'stopping') {
      // Stopping before the display hears it: a stream that fails as it is told ends the recording, for good.
      active = { ...a, phase: 'stopping', capture: null };
      a.capture?.stop();
      // Nothing more goes to the display: the microphone closes now (the camera once the recording ends).
      stopTracks(a.mic);
      viewer?.stopRecording(a.id);
    }
    // Late, the recording is still this one: only its end settles `done`, and none starts before it ended.
    await awaitAtMost(a.done, 15_000, () => {
      if (active && !active.written) recordingEnded(false);
    });
    return a.done;
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
    if (!gone) onExit('closed');
  }
</script>

{#snippet transfers(title: string, list: Transfer[])}
  {#if list.length > 0}
    <ul class="flex flex-wrap gap-2" aria-label={title}>
      {#each list as item, i (i)}<li class="rounded-full border border-line bg-paper px-3 py-1.5 text-sm"><span class="font-semibold">{item.name}</span> <span class="text-muted">{item.detail()}</span></li>{/each}
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
    sound={features?.sound === true}
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

{#snippet overlays()}
  {#if batchShown}<TransferCard transfer={batchShown} width={screenWidth} height={screenHeight} />{/if}
  {#if camStream && screenWidth > 0}
    <!-- The camera where the recording puts it, live: what you see is what is recorded. -->
    <CameraBubble stream={camStream} layout={cameraLayout} width={screenWidth} height={screenHeight} onLayout={layoutChanged} />
  {/if}
{/snippet}

{#snippet extras()}
  {#if fileRequest && !(demo && viewOnly)}
    <div class="flex flex-wrap items-center gap-2">
      <Button variant="primary" icon={ICONS.upload} onclick={() => fileRequest?.choose()}>{t('session.files.choose')}</Button>
      <Button onclick={() => fileRequest?.cancel()}>{t('session.files.cancel')}</Button>
    </div>
  {/if}
  {#if copyOffered}<Button icon={ICONS.copy} onclick={copy}>{t('session.copy.offer')}</Button>{/if}
  <span class="text-sm font-semibold empty:hidden {demo ? 'sr-only' : 'rounded-full bg-paper px-3.5 py-1.5 ring-1 ring-line'}" role="status">{copied ? t('session.copy.done') : ''}</span>
  <span class="sr-only" role="status">{fileRequest ? t('session.files.asked') : ''}</span>
  <div class="sr-only" role="log" aria-live="polite">{#each transferSaid as entry (entry.batch)}<p>{entry.text()}</p>{/each}</div>
  {#if !demo}{@render transfers(t('session.uploads'), uploads)}{/if}
  {@render transfers(t('session.downloads'), downloads)}
{/snippet}

<SessionBar
  {holder}
  {disconnected}
  {stats}
  {sound}
  bind:level
  onMute={mute}
  onVolume={(v) => viewer?.setVolume(v)}
  onTake={take}
  pinned={recordOpen || active !== null}
  record={recorder}
  recording={demo || (features?.recording ?? false)}
  hearing={demo || (features?.sound ?? false)}
  onGiveBack={giveBack}
  note={mine ? t('session.control.yours.note') : ''}
  name={info?.profile?.name ?? null}
  onSessions={demo ? undefined : toSessions}
  onClose={demo ? undefined : close}
  demo={demo
    ? {
        sites: demoSites,
        site: demoSite,
        switching,
        onPick: switchTo,
        picker: framer === null,
        autoFailed,
        viewOnly,
        onViewOnly: setViewOnly,
        left,
        total: demoSeconds,
        onLeave: signOut,
      }
    : undefined}
/>
<!-- The screen fills the window under the bar, edge to edge (an administrator's session takes the window's shape, see
     Sessions); in the demo it sits in a ring that says who holds control, and framed by a page it fills the frame: that
     page draws the frame's edge, only the ring shows. -->
<main class="fixed inset-0 grid place-items-center bg-wash {framer ? 'p-[3px]' : demo ? 'p-2' : ''}">
  <div class="relative" bind:clientWidth={screenWidth} bind:clientHeight={screenHeight}>
    <div
      class="relative overflow-hidden bg-[#0f1b33] {pictured ? '' : 'opacity-0'} {!demo
        ? '[&_canvas[data-screen]]:w-[min(100vw,calc(100vh*var(--screen-ratio,1.6)))]! [&_canvas[data-screen]]:max-w-none!'
        : `rounded-xl transition ${mine ? (viewOnly ? 'ring-2 ring-bad' : 'ring-2 ring-accent') : 'ring-1 ring-line'} ${framer
            ? '[&_canvas]:max-h-[calc(100vh-6px)]! [&_canvas]:max-w-[calc(100vw-6px)]!'
            : 'shadow-[0_30px_80px_-40px_rgb(20_40_75/0.55)] [&_canvas]:max-h-[calc(100vh-1rem)]! [&_canvas]:max-w-[calc(100vw-1rem)]!'}`}"
      bind:this={screen}
    ></div>
    {@render overlays()}
  </div>
  {#if problem}
    <div class="absolute inset-0 grid place-items-center bg-wash p-4" role="alert">
      <div class="w-full max-w-sm rounded-2xl bg-paper p-7 text-center shadow-[0_24px_60px_-24px_rgb(20_40_75/0.45)]">
        <span class="mx-auto grid size-12 place-items-center rounded-full bg-accent-soft text-accent">
          <Icon d={problem === 'lost' ? ICONS.offline : ICONS.slow} class="size-6" />
        </span>
        <h1 class="mt-4 text-lg font-semibold">{problem === 'lost' ? t('session.problem.lost') : t('session.problem.slow')}</h1>
        <p class="mt-2 text-sm text-muted">{problem === 'lost' ? t('session.problem.lost.help') : t('session.problem.slow.help')}</p>
        <div class="mt-6 flex flex-col gap-2">
          <Button variant="primary" icon={ICONS.retry} onclick={() => location.reload()}>{t('session.problem.reload')}</Button>
          {#if !demo}<Button icon={ICONS.sessions} onclick={toSessions}>{t('session.problem.sessions')}</Button>{/if}
        </div>
      </div>
    </div>
  {:else if !pictured}
    <div class="shimmer absolute inset-0" role="status"><span class="sr-only">{t('session.opening')}</span></div>
  {/if}
  {#if switching !== null}
    <div class="absolute inset-0 grid place-items-center bg-wash/80 backdrop-blur-sm" role="status">
      <p class="flex items-center gap-3 rounded-2xl bg-paper px-5 py-3.5 font-semibold shadow-[0_24px_60px_-24px_rgb(20_40_75/0.45)]">
        <span class="size-4 animate-spin rounded-full border-2 border-accent border-t-transparent" aria-hidden="true"></span>
        {t('session.switching', { demo: t(demoOf(switching).title) })}
      </p>
    </div>
  {/if}
</main>
<div class="pointer-events-none fixed inset-x-0 bottom-4 z-20 flex flex-col items-center gap-2 px-4 [&>*]:pointer-events-auto">
  <div class="flex flex-wrap items-center justify-center gap-3 empty:hidden">{@render extras()}</div>
  {#if recNotice}<p class="rounded-full bg-paper px-4 py-2 text-sm font-semibold shadow-[0_18px_44px_-14px_rgb(20_40_75/0.45)] ring-1 ring-line" role="status">{recNotice()}</p>{/if}

  {#if error}<p class="rounded-full bg-bad px-4 py-2 text-sm font-semibold text-white" role="alert">{error()}</p>{/if}
</div>
