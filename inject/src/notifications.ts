// `notifications` (spec 07 §1.1): replaces `Notification` so the core
// decides and macOS shows them. Page world, document start, main frame.

const ask = (type: string, payload: unknown) =>
  document.dispatchEvent(new CustomEvent("mado:request", { detail: { type, payload } }));

let permission: NotificationPermission = "default";
const waiting: ((p: NotificationPermission) => void)[] = [];
const shown = new Map<number, MadoNotification>();
let next = 1;

document.addEventListener("mado:reply", (e) => {
  let m: { type?: string; permission?: string; id?: number };
  try {
    m = JSON.parse((e as CustomEvent).detail);
  } catch {
    return;
  }
  if (
    m.type === "notificationPermission" &&
    (m.permission === "granted" || m.permission === "denied" || m.permission === "default")
  ) {
    permission = m.permission;
    for (const w of waiting.splice(0)) w(permission);
  } else if (m.type === "notificationClick" && typeof m.id === "number") {
    shown.get(m.id)?.dispatchEvent(new Event("click"));
  }
});

class MadoNotification extends EventTarget {
  static get permission() {
    return permission;
  }
  static requestPermission(
    callback?: (p: NotificationPermission) => void,
  ): Promise<NotificationPermission> {
    return new Promise((resolve) => {
      waiting.push((p) => {
        callback?.(p);
        resolve(p);
      });
      ask("notificationRequest", {});
    });
  }
  readonly title: string;
  readonly body: string;
  readonly tag: string;
  onclick: ((e: Event) => void) | null = null;
  #id: number;
  constructor(title: string, options: NotificationOptions = {}) {
    super();
    this.title = String(title);
    this.body = String(options.body ?? "");
    this.tag = String(options.tag ?? "");
    this.#id = next++;
    this.addEventListener("click", (e) => this.onclick?.(e));
    if (permission !== "granted") return;
    shown.set(this.#id, this);
    ask("notificationShow", {
      id: this.#id,
      title: this.title.slice(0, 256),
      body: this.body.slice(0, 1024),
      tag: this.tag.slice(0, 256),
    });
  }
  close() {
    shown.delete(this.#id);
    ask("notificationClose", { id: this.#id });
  }
}

Object.defineProperty(window, "Notification", {
  value: MadoNotification,
  writable: true,
  configurable: true,
});

export {};
