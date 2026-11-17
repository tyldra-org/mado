// `webauthn` (spec 11 §8): Mado Passwords as a passkey authenticator.
// Replaces `navigator.credentials.create` and `get` for public-key
// requests; the core decides, asks the person, and answers through the
// relay. Anything Mado does not handle falls through to the original API.
// Page world, document start, main frame.

type Reply = {
  type?: string;
  id?: number;
  fallThrough?: boolean;
  error?: string;
  message?: string;
  credential?: Record<string, unknown>;
};

const credentials = navigator.credentials;
const originalCreate = credentials.create.bind(credentials);
const originalGet = credentials.get.bind(credentials);
const Pkc = globalThis.PublicKeyCredential;
const waiting = new Map<number, (r: Reply) => void>();
let next = 1;

const ask = (type: string, payload: Record<string, unknown>) =>
  document.dispatchEvent(new CustomEvent("mado:request", { detail: { type, payload } }));

document.addEventListener("mado:reply", (e) => {
  let m: Reply;
  try {
    m = JSON.parse((e as CustomEvent).detail);
  } catch {
    return;
  }
  if (m.type !== "webauthnResult" || typeof m.id !== "number") return;
  const done = waiting.get(m.id);
  waiting.delete(m.id);
  done?.(m);
});

const bytes = (b: BufferSource): Uint8Array =>
  b instanceof ArrayBuffer
    ? new Uint8Array(b)
    : new Uint8Array(b.buffer, b.byteOffset, b.byteLength);

function b64(b: BufferSource): string {
  let s = "";
  for (const x of bytes(b)) s += String.fromCharCode(x);
  return btoa(s).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
}

function unb64(s: string): ArrayBuffer {
  const bin = atob(s.replace(/-/g, "+").replace(/_/g, "/"));
  const out = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
  return out.buffer;
}

/** Sends a request and waits; an abort cancels it in the core too. */
function request(
  type: string,
  payload: Record<string, unknown>,
  signal?: AbortSignal | null,
): Promise<Reply> {
  const id = next++;
  return new Promise((resolve, reject) => {
    if (signal?.aborted) {
      reject(new DOMException("The operation was aborted.", "AbortError"));
      return;
    }
    waiting.set(id, resolve);
    signal?.addEventListener(
      "abort",
      () => {
        if (!waiting.delete(id)) return;
        ask("webauthnCancel", { id });
        reject(new DOMException("The operation was aborted.", "AbortError"));
      },
      { once: true },
    );
    ask(type, { id, ...payload });
  });
}

function fail(r: Reply): never {
  throw new DOMException(
    r.message ?? "The operation either timed out or was not allowed.",
    r.error ?? "NotAllowedError",
  );
}

const define = (proto: object, props: Record<string, unknown>) =>
  Object.create(
    proto,
    Object.fromEntries(Object.entries(props).map(([k, value]) => [k, { value, enumerable: true }])),
  );

function credential(
  c: Record<string, unknown>,
  attestation: boolean,
  wantsCredProps: boolean,
): Credential {
  const raw = unb64(c.id as string);
  const clientDataJSON = unb64(c.clientDataJSON as string);
  const response = attestation
    ? define(globalThis.AuthenticatorAttestationResponse?.prototype ?? Object.prototype, {
        clientDataJSON,
        attestationObject: unb64(c.attestationObject as string),
        getAuthenticatorData: () => unb64(c.authenticatorData as string),
        getPublicKey: () => unb64(c.publicKey as string),
        getPublicKeyAlgorithm: () => -7,
        getTransports: () => ["internal", "hybrid"],
      })
    : define(globalThis.AuthenticatorAssertionResponse?.prototype ?? Object.prototype, {
        clientDataJSON,
        authenticatorData: unb64(c.authenticatorData as string),
        signature: unb64(c.signature as string),
        userHandle: c.userHandle ? unb64(c.userHandle as string) : null,
      });
  const json = () => ({
    id: c.id,
    rawId: c.id,
    type: "public-key",
    authenticatorAttachment: "platform",
    clientExtensionResults: attestation && wantsCredProps ? { credProps: { rk: true } } : {},
    response: attestation
      ? {
          clientDataJSON: c.clientDataJSON,
          attestationObject: c.attestationObject,
          authenticatorData: c.authenticatorData,
          publicKey: c.publicKey,
          publicKeyAlgorithm: -7,
          transports: ["internal", "hybrid"],
        }
      : {
          clientDataJSON: c.clientDataJSON,
          authenticatorData: c.authenticatorData,
          signature: c.signature,
          userHandle: c.userHandle ?? null,
        },
  });
  return define(Pkc?.prototype ?? Object.prototype, {
    id: c.id,
    rawId: raw,
    type: "public-key",
    authenticatorAttachment: "platform",
    response,
    getClientExtensionResults: () =>
      attestation && wantsCredProps ? { credProps: { rk: true } } : {},
    toJSON: json,
  }) as Credential;
}

credentials.create = async function create(options?: CredentialCreationOptions) {
  const pk = options?.publicKey;
  if (!pk) return originalCreate(options);
  const r = await request(
    "webauthnCreate",
    {
      rpId: pk.rp.id ?? null,
      rpName: String(pk.rp.name ?? "").slice(0, 256),
      userId: b64(pk.user.id),
      userName: String(pk.user.name ?? "").slice(0, 256),
      userDisplayName: String(pk.user.displayName ?? "").slice(0, 256),
      challenge: b64(pk.challenge),
      algs: (pk.pubKeyCredParams ?? []).map((p) => p.alg).slice(0, 16),
      exclude: (pk.excludeCredentials ?? []).map((c) => b64(c.id)).slice(0, 64),
    },
    options?.signal,
  );
  if (r.fallThrough) return originalCreate(options);
  if (!r.credential) fail(r);
  const wantsCredProps = (pk.extensions as { credProps?: boolean } | undefined)?.credProps === true;
  return credential(r.credential, true, wantsCredProps);
};

credentials.get = async function get(options?: CredentialRequestOptions) {
  const pk = options?.publicKey;
  if (!pk) return originalGet(options);
  const r = await request(
    "webauthnGet",
    {
      rpId: pk.rpId ?? null,
      challenge: b64(pk.challenge),
      allow: (pk.allowCredentials ?? []).map((c) => b64(c.id)).slice(0, 64),
      conditional: options?.mediation === "conditional",
    },
    options?.signal,
  );
  if (r.fallThrough) return originalGet(options);
  if (!r.credential) fail(r);
  return credential(r.credential, false, false);
};

if (Pkc) {
  // Mado is a platform authenticator that checks the person (Touch ID).
  Pkc.isUserVerifyingPlatformAuthenticatorAvailable = () => Promise.resolve(true);
}

export {};
