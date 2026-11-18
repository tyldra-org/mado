// Fixture state for the gallery (M6): one window with pinned, grouped,
// agent and sleeping tabs, and every setting the screens read. Not shipped.
import type { State } from "../generated/State";

export const W = "01a1d7aa-0000-7000-8000-000000000001";
const id = (n: number) => `01a1d7aa-0000-7000-8000-${String(n).padStart(12, "0")}`;
export const TABS: string[] = [1, 2, 3, 4, 5, 6, 7, 8].map(id);
const T = (n: number) => TABS[n] as string;
const [G1, G2] = [id(101), id(102)];
const TASK = id(201);

const tab = (n: number, title: string, url: string, extra: Record<string, unknown> = {}) => ({
  id: T(n),
  windowId: W,
  groupId: null,
  pinned: false,
  title,
  customTitle: null,
  url,
  favicon: null,
  life: "live",
  audio: { playing: false, muted: false },
  loading: false,
  agent: null,
  security: { level: "secure" },
  displayUrl: { text: new URL(url).hostname.replace(/^www\./, ""), host: new URL(url).hostname },
  canGoBack: true,
  canGoForward: false,
  progress: null,
  ...extra,
});

export interface Options {
  theme: string;
  layout: string;
  density: string;
  prompts?: Record<string, unknown>;
  notices?: unknown[];
  extra?: Record<string, unknown>;
}

export function state(o: Options): State {
  return {
    rev: 1,
    firstRun: false,
    profile: {
      id: "p",
      name: "Personal",
      color: "blue",
      avatar: "",
      isPrivate: false,
      isGuest: false,
    },
    windows: {
      [W]: {
        id: W,
        kind: "normal",
        layout: o.layout,
        tabOrder: TABS,
        activeTabId: T(2),
        overlay: null,
        locked: false,
        sidebar: { collapsed: false, width: 240 },
        findBar: null,
        focus: "page",
        status: null,
      },
    },
    tabs: {
      [T(0)]: tab(0, "Mail", "https://mail.example/", { pinned: true }),
      [T(1)]: tab(1, "Calendar", "https://calendar.example/", { pinned: true }),
      [T(2)]: tab(2, "The quiet web: notes on browsing", "https://journal.example/quiet-web"),
      [T(3)]: tab(3, "Design tokens · Docs", "https://docs.example/tokens", { groupId: G1 }),
      [T(4)]: tab(4, "Issue 239 · tyldra-org/mado", "https://github.com/tyldra-org/mado/pull/239", {
        groupId: G1,
        audio: { playing: true, muted: false },
      }),
      [T(5)]: tab(5, "Flights to Lisbon", "https://flights.example/lis", {
        groupId: G2,
        agent: { taskId: TASK, takenOver: false },
      }),
      [T(6)]: tab(
        6,
        "A long article title that keeps going well past the width of the sidebar",
        "https://news.example/a",
        {
          life: "sleeping",
        },
      ),
      [T(7)]: tab(7, "Recipes", "https://food.example/", { life: "sleeping" }),
    },
    groups: {
      [G1]: {
        id: G1,
        windowId: W,
        title: "Mado",
        color: "blue",
        collapsed: false,
        saved: true,
        agentTaskId: null,
      },
      [G2]: {
        id: G2,
        windowId: W,
        title: "Trip to Lisbon",
        color: "purple",
        collapsed: false,
        saved: false,
        agentTaskId: TASK,
      },
    },
    prompts: o.prompts ?? {},
    notices: o.notices ?? [],
    downloads: {},
    settings: {
      values: {
        "appearance.theme": o.theme,
        "appearance.layout": o.layout,
        "appearance.density": o.density,
        "appearance.bookmarksBar": false,
      },
      changed: [],
    },
    features: { ai: true, agents: true },
    ai: {
      enabled: true,
      connections: {},
      roles: {},
      tasks: {
        [TASK]: {
          id: TASK,
          title: "Find a flight to Lisbon on Friday",
          status: "running",
          step: "open flights.example",
          groupId: G2,
          costMicros: 12000,
        },
      },
    },
    extensions: {},
    blocker: { enabled: true },
    ...o.extra,
  } as unknown as State;
}
