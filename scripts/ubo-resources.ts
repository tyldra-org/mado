// Builds adblock-rust resources from uBlock Origin's scriptlets (spec 10 §2,
// §6): each scriptlet's function source, its aliases and dependencies, and
// `requiresTrust` as permission bit 1, so only trusted lists may use it.
// lists.yml runs it on a pinned uBlock Origin checkout and publishes the
// result, signed, as lists/ubo-resources.json. uBlock Origin is GPL-3.0; the
// file is fetched at runtime and never bundled.
//
//   bun scripts/ubo-resources.ts <uBlock checkout> > ubo-resources.json

export interface Scriptlet {
  name: string;
  aliases?: string[];
  fn: (...args: never[]) => unknown;
  dependencies?: string[];
  requiresTrust?: boolean;
}

export interface Resource {
  name: string;
  aliases: string[];
  kind: { mime: "fn/javascript" | "application/javascript" };
  content: string;
  dependencies?: string[];
  permission?: number;
}

/** One scriptlet as an adblock-rust `Resource`. */
export function toResource(s: Scriptlet): Resource {
  const source = s.fn.toString();
  // adblock-rust calls an injectable scriptlet by its function's name;
  // dependencies (`.fn`) are only pasted in, so classes and async ones do.
  if (!s.name.endsWith(".fn") && !/^function\s+[\w$]+\s*\(/.test(source)) {
    throw new Error(`${s.name}: not a named function`);
  }
  const r: Resource = {
    name: s.name,
    aliases: s.aliases ?? [],
    // adblock-rust injects only `application/javascript` resources (called
    // by function name); `fn/javascript` ones are dependencies.
    kind: { mime: s.name.endsWith(".fn") ? "fn/javascript" : "application/javascript" },
    content: Buffer.from(source).toString("base64"),
  };
  if (s.dependencies && s.dependencies.length > 0) r.dependencies = s.dependencies;
  if (s.requiresTrust) r.permission = 1;
  return r;
}

/** Dependencies before what uses them: adblock-rust drops a resource whose
 * dependency it has not seen yet. */
export function ordered(resources: Resource[]): Resource[] {
  const byName = new Map(resources.map((r) => [r.name, r]));
  const done = new Set<string>();
  const out: Resource[] = [];
  const visit = (r: Resource, path: Set<string>) => {
    if (done.has(r.name)) return;
    if (path.has(r.name)) throw new Error(`${r.name}: dependency cycle`);
    path.add(r.name);
    for (const d of r.dependencies ?? []) {
      const dep = byName.get(d);
      if (!dep) throw new Error(`${r.name}: unknown dependency ${d}`);
      visit(dep, path);
    }
    done.add(r.name);
    out.push(r);
  };
  for (const r of resources) visit(r, new Set());
  return out;
}

if (import.meta.main) {
  const root = process.argv[2];
  if (!root) {
    console.error("usage: bun scripts/ubo-resources.ts <uBlock checkout>");
    process.exit(2);
  }
  const mod = (await import(`${root}/src/js/resources/scriptlets.js`)) as {
    builtinScriptlets: Scriptlet[];
  };
  const resources = ordered(mod.builtinScriptlets.map(toResource));
  if (resources.length < 50) throw new Error(`only ${resources.length} scriptlets`);
  console.log(JSON.stringify(resources));
}
