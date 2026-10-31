// The features page's cards: the two the landing first showed, then the rest in groups. Each card's drawing, and its
// words, features.<card>.title and .text; each group's name, features.group.<group>.
import type { Component } from 'svelte';
import Takeover from './Takeover.svelte';
import Vault from './Vault.svelte';
import Mcp from './Mcp.svelte';
import Api from './Api.svelte';
import Sees from './Sees.svelte';
import Steps from './Steps.svelte';
import Watch from './Watch.svelte';
import Own from './Own.svelte';
import Parallel from './Parallel.svelte';
import Duration from './Duration.svelte';
import Lend from './Lend.svelte';
import Files from './Files.svelte';
import Engine from './Engine.svelte';
import Live from './Live.svelte';
import Sound from './Sound.svelte';
import Drop from './Drop.svelte';
import Picker from './Picker.svelte';
import Downloads from './Downloads.svelte';
import Clipboard from './Clipboard.svelte';
import Keyboard from './Keyboard.svelte';
import Recording from './Recording.svelte';
import Screen from './Screen.svelte';
import Together from './Together.svelte';
import Connection from './Connection.svelte';
import Languages from './Languages.svelte';
import Redact from './Redact.svelte';
import Sandbox from './Sandbox.svelte';
import Private from './Private.svelte';
import Stealth from './Stealth.svelte';
import Proxies from './Proxies.svelte';
import Signin from './Signin.svelte';
import Switches from './Switches.svelte';
import Fresh from './Fresh.svelte';
import Pinned from './Pinned.svelte';
import Install from './Install.svelte';
import Tuning from './Tuning.svelte';
import Port from './Port.svelte';
import Light from './Light.svelte';
import Fees from './Fees.svelte';
import Open from './Open.svelte';
import Closed from './Closed.svelte';
import Sdk from './Sdk.svelte';
import Demo from './Demo.svelte';

export type Card = { id: string; art: Component };

/** The first two, side by side above the groups. */
export const FIRST: Card[] = [
  { id: 'takeover', art: Takeover },
  { id: 'vault', art: Vault },
];

export const GROUPS: { id: string; cards: Card[] }[] = [
  {
    id: 'agents',
    cards: [
      { id: 'mcp', art: Mcp },
      { id: 'api', art: Api },
      { id: 'sees', art: Sees },
      { id: 'steps', art: Steps },
      { id: 'watch', art: Watch },
      { id: 'own', art: Own },
      { id: 'parallel', art: Parallel },
      { id: 'duration', art: Duration },
      { id: 'lend', art: Lend },
      { id: 'files', art: Files },
    ],
  },
  {
    id: 'you',
    cards: [
      { id: 'engine', art: Engine },
      { id: 'live', art: Live },
      { id: 'sound', art: Sound },
      { id: 'drop', art: Drop },
      { id: 'picker', art: Picker },
      { id: 'downloads', art: Downloads },
      { id: 'clipboard', art: Clipboard },
      { id: 'keyboard', art: Keyboard },
      { id: 'record', art: Recording },
      { id: 'screen', art: Screen },
      { id: 'together', art: Together },
      { id: 'connection', art: Connection },
      { id: 'languages', art: Languages },
    ],
  },
  {
    id: 'security',
    cards: [
      { id: 'redact', art: Redact },
      { id: 'sandbox', art: Sandbox },
      { id: 'private', art: Private },
      { id: 'stealth', art: Stealth },
      { id: 'proxy', art: Proxies },
      { id: 'signin', art: Signin },
      { id: 'switches', art: Switches },
      { id: 'fresh', art: Fresh },
      { id: 'pinned', art: Pinned },
    ],
  },
  {
    id: 'server',
    cards: [
      { id: 'install', art: Install },
      { id: 'tuning', art: Tuning },
      { id: 'port', art: Port },
      { id: 'light', art: Light },
      { id: 'fees', art: Fees },
      { id: 'open', art: Open },
      { id: 'closed', art: Closed },
      { id: 'sdk', art: Sdk },
      { id: 'demo', art: Demo },
    ],
  },
];
