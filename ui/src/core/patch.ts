// Applies RFC 6902 operations to the state document (mado-docs/spec/01
// section 3.1). The core only sends add, remove, replace and move; paths are
// JSON Pointers with ~0 and ~1 escapes.
import type { PatchOp } from "../generated/PatchOp";

type Container = Record<string, unknown> | unknown[];

function tokens(pointer: string): string[] {
  if (pointer === "") return [];
  if (!pointer.startsWith("/")) throw new Error(`not a JSON Pointer: ${pointer}`);
  return pointer
    .slice(1)
    .split("/")
    .map((t) => t.replaceAll("~1", "/").replaceAll("~0", "~"));
}

function isContainer(value: unknown): value is Container {
  return typeof value === "object" && value !== null;
}

/** The parent of the target and the last token, copying every container on the way. */
function walk(root: Container, path: string[]): [Container, string] {
  let node: Container = root;
  for (const token of path.slice(0, -1)) {
    const child: unknown = Array.isArray(node) ? node[Number(token)] : node[token];
    if (!isContainer(child)) throw new Error(`no container at ${token}`);
    const copy: Container = Array.isArray(child) ? [...child] : { ...child };
    if (Array.isArray(node)) node[Number(token)] = copy;
    else node[token] = copy;
    node = copy;
  }
  const last = path.at(-1);
  if (last === undefined) throw new Error("the root cannot be replaced by an operation");
  return [node, last];
}

function read(root: unknown, path: string[]): unknown {
  let node = root;
  for (const token of path) {
    if (!isContainer(node)) throw new Error(`no value at ${token}`);
    node = Array.isArray(node) ? node[Number(token)] : node[token];
  }
  return node;
}

function put(parent: Container, key: string, value: unknown, insert: boolean) {
  if (Array.isArray(parent)) {
    const index = key === "-" ? parent.length : Number(key);
    if (insert) parent.splice(index, 0, value);
    else parent[index] = value;
  } else {
    parent[key] = value;
  }
}

function drop(parent: Container, key: string) {
  if (Array.isArray(parent)) parent.splice(Number(key), 1);
  else delete parent[key];
}

/**
 * Returns a new document with `ops` applied. Containers on changed paths are
 * copied, so unchanged branches keep their identity and the rest of the UI
 * does not re-render them.
 */
export function applyPatch<T>(doc: T, ops: readonly PatchOp[]): T {
  if (!isContainer(doc)) throw new Error("the document must be an object");
  const root: Container = Array.isArray(doc) ? [...doc] : { ...doc };
  for (const op of ops) {
    const path = tokens(op.path);
    switch (op.op) {
      case "add": {
        const [parent, key] = walk(root, path);
        put(parent, key, op.value, true);
        break;
      }
      case "replace": {
        const [parent, key] = walk(root, path);
        put(parent, key, op.value, false);
        break;
      }
      case "remove": {
        const [parent, key] = walk(root, path);
        drop(parent, key);
        break;
      }
      case "move": {
        const from = tokens(op.from);
        const value = read(root, from);
        const [source, sourceKey] = walk(root, from);
        drop(source, sourceKey);
        const [target, targetKey] = walk(root, path);
        put(target, targetKey, value, true);
        break;
      }
    }
  }
  return root as T;
}
